package com.eclipseos.maos

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.util.Base64
import android.util.Log
import java.io.ByteArrayOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Print da tela inteira por MediaProjection.
 *
 * O `takeScreenshot` da acessibilidade só existe do Android 11 em diante, e a
 * central é 10. A MediaProjection captura a tela já composta, então o mapa
 * (WebGL) aparece — o print do Eclipse DEV saía com o mapa preto.
 *
 * O Android 10 exige um foreground service do tipo mediaProjection rodando
 * ANTES de entregar a projeção; por isso o resultado do consentimento vai para
 * cá em vez de ser usado na activity.
 */
class Captura : Service() {

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val canal = "captura"
        val nm = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(NotificationChannel(canal, "Print da tela", NotificationManager.IMPORTANCE_MIN))
        }
        val aviso = (if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this, canal) else Notification.Builder(this))
            .setSmallIcon(android.R.drawable.ic_menu_camera)
            .setContentTitle("Eclipse Mãos")
            .setContentText("O Claude pode tirar print da tela")
            .build()
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(1, aviso, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } else {
            startForeground(1, aviso)
        }

        val dados = intent?.getParcelableExtra<Intent>("dados")
        val codigo = intent?.getIntExtra("codigo", 0) ?: 0
        if (dados != null) {
            val mpm = getSystemService(MediaProjectionManager::class.java)
            projecao?.stop()
            projecao = mpm.getMediaProjection(codigo, dados)?.also { p ->
                p.registerCallback(object : MediaProjection.Callback() {
                    override fun onStop() {
                        if (projecao === p) projecao = null
                        Log.i("maos", "projeção encerrada")
                    }
                }, Handler(mainLooper))
            }
            Log.i("maos", "projeção ${if (projecao != null) "ativa" else "recusada"}")
        }
        if (projecao == null) stopSelf()
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        projecao?.stop()
        projecao = null
        super.onDestroy()
    }

    companion object {
        @Volatile var projecao: MediaProjection? = null
            private set

        private val linha = HandlerThread("captura").apply { start() }
        private val handler = Handler(linha.looper)

        val ativa get() = projecao != null

        /**
         * Garante a permissão de print. Se não houver, abre o pedido do sistema
         * e arma o piloto da acessibilidade para tocar em "Iniciar agora".
         */
        fun garantir(ctx: Context) {
            if (ativa) return
            Acessibilidade.consentirCapturaAte = System.currentTimeMillis() + 10_000
            ctx.startActivity(Intent(ctx, PedirCaptura::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            val limite = System.currentTimeMillis() + 10_000
            while (!ativa && System.currentTimeMillis() < limite) Thread.sleep(200)
            if (ativa) {
                // A janela de permissão ainda está sumindo; sem esperar, ela
                // sai no primeiro print.
                Thread.sleep(700)
                return
            }
            throw Falha(
                "Sem permissão de print: o pedido do Android abriu e ninguém aceitou. " +
                    "Veja a tela com `tela` e toque em \"Iniciar agora\".",
            )
        }

        /**
         * Cria um display virtual só para este print e o desfaz em seguida. Um
         * display permanente acumularia quadros velhos no ImageReader quando a
         * tela fica parada, e o print sairia atrasado.
         */
        fun tirar(maxLargura: Int, qualidade: Int): Pair<String, String> {
            val p = projecao ?: throw Falha("Sem permissão de print.")
            val (w, h) = Ferramentas.tamanhoTela()
            val dpi = Ferramentas.densidade()
            val leitor = ImageReader.newInstance(w, h, PixelFormat.RGBA_8888, 2)
            val pronto = CountDownLatch(1)
            var bitmap: Bitmap? = null
            leitor.setOnImageAvailableListener({ r ->
                if (bitmap != null) return@setOnImageAvailableListener
                val img = r.acquireLatestImage() ?: return@setOnImageAvailableListener
                try {
                    val plano = img.planes[0]
                    val larguraComSobra = plano.rowStride / plano.pixelStride
                    val cheio = Bitmap.createBitmap(larguraComSobra, h, Bitmap.Config.ARGB_8888)
                    cheio.copyPixelsFromBuffer(plano.buffer)
                    bitmap = if (larguraComSobra == w) cheio else Bitmap.createBitmap(cheio, 0, 0, w, h)
                } finally {
                    img.close()
                }
                pronto.countDown()
            }, handler)
            val display = p.createVirtualDisplay(
                "maos-print", w, h, dpi,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
                leitor.surface, null, handler,
            )
            try {
                if (!pronto.await(3, TimeUnit.SECONDS)) throw Falha("O Android não entregou nenhum quadro da tela em 3 s.")
            } finally {
                display?.release()
                leitor.close()
            }
            var img = bitmap!!
            var legenda = "Tela ${w}x$h."
            if (w > maxLargura) {
                val escala = maxLargura.toFloat() / w
                img = Bitmap.createScaledBitmap(img, maxLargura, (h * escala).toInt(), true)
                legenda += " Imagem reduzida para ${img.width}x${img.height}: multiplique as coordenadas da imagem por ${"%.3f".format(1 / escala)} para tocar."
            } else {
                legenda += " As coordenadas da imagem são as da tela."
            }
            val saida = ByteArrayOutputStream()
            img.compress(Bitmap.CompressFormat.JPEG, qualidade, saida)
            return Base64.encodeToString(saida.toByteArray(), Base64.NO_WRAP) to legenda
        }
    }
}

/** Activity invisível que só pede o consentimento de print e some. */
class PedirCaptura : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val mpm = getSystemService(MediaProjectionManager::class.java)
        startActivityForResult(mpm.createScreenCaptureIntent(), 1)
    }

    @Deprecated("API antiga, mas é a que existe no Android 10")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (resultCode == RESULT_OK && data != null) {
            val i = Intent(this, Captura::class.java).putExtra("codigo", resultCode).putExtra("dados", data)
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(i) else startService(i)
        }
        finish()
    }
}
