package com.eclipseos.maos

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import okhttp3.OkHttpClient
import okhttp3.Request
import okio.ByteString
import org.json.JSONObject
import java.io.OutputStream
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Instala APK sem ninguém no carro.
 *
 * O APK chega de dois jeitos: em pedaços pelo próprio WebSocket (quando o
 * Claude envia um arquivo do Mac com `scripts/carro-apk.mjs`, já que o carro
 * não alcança o Mac) ou baixado de uma URL. Os bytes vão direto para uma
 * sessão do PackageInstaller, sem arquivo intermediário.
 *
 * O Android sempre pede confirmação para instalar app que não veio da loja; o
 * piloto da acessibilidade toca em "Instalar" por nós.
 */
object Instalador {
    private class Recebimento(val id: String, val sessao: PackageInstaller.Session, val saida: OutputStream) {
        var recebidos = 0L
        var ultimoAck = 0L
    }

    @Volatile private var atual: Recebimento? = null
    private val esperando = ConcurrentHashMap<Int, (Resultado) -> Unit>()

    // --- pelo WebSocket -------------------------------------------------------

    fun inicio(id: String, tamanho: Long) {
        atual?.let { cancelar(it, "outro APK começou a chegar") }
        try {
            val (sessao, saida) = abrirSessao(tamanho)
            atual = Recebimento(id, sessao, saida)
        } catch (e: Exception) {
            Conexao.responder(id, Resultado.erro("Não deu para abrir a instalação: ${e.message}"))
        }
    }

    fun pedaco(bytes: ByteString) {
        val r = atual ?: return
        try {
            r.saida.write(bytes.toByteArray())
            r.recebidos += bytes.size
            // O relay só manda mais quando sabe que gravamos: sem isso ele
            // empurra o APK inteiro de uma vez para a memória.
            if (r.recebidos - r.ultimoAck >= 1024 * 1024) {
                r.ultimoAck = r.recebidos
                Conexao.enviar(JSONObject().put("tipo", "apk_ack").put("id", r.id).put("recebidos", r.recebidos).toString())
            }
        } catch (e: Exception) {
            cancelar(r, "falhou gravando: ${e.message}")
        }
    }

    fun fim(id: String) {
        val r = atual?.takeIf { it.id == id } ?: return
        atual = null
        val resultado = try {
            r.sessao.fsync(r.saida)
            r.saida.close()
            confirmar(r.sessao, "${r.recebidos / 1024} KB recebidos")
        } catch (e: Exception) {
            r.sessao.abandon()
            Resultado.erro("Falhou instalando: ${e.message}")
        }
        Conexao.responder(id, resultado)
    }

    private fun cancelar(r: Recebimento, motivo: String) {
        atual = null
        try { r.saida.close() } catch (_: Exception) {}
        r.sessao.abandon()
        Conexao.responder(r.id, Resultado.erro(motivo))
    }

    // --- por URL --------------------------------------------------------------

    fun deUrl(url: String): Resultado {
        Eventos.anotar("instalar: baixando $url")
        val resp = OkHttpClient.Builder().readTimeout(2, TimeUnit.MINUTES).build()
            .newCall(Request.Builder().url(url).build()).execute()
        if (!resp.isSuccessful) throw Falha("Download falhou: HTTP ${resp.code}")
        val corpo = resp.body ?: throw Falha("Download veio vazio.")
        val (sessao, saida) = abrirSessao(corpo.contentLength())
        try {
            val n = corpo.byteStream().use { it.copyTo(saida) }
            sessao.fsync(saida)
            saida.close()
            return confirmar(sessao, "${n / 1024} KB baixados")
        } catch (e: Exception) {
            sessao.abandon()
            throw e
        }
    }

    // --- sessão ---------------------------------------------------------------

    private fun abrirSessao(tamanho: Long): Pair<PackageInstaller.Session, OutputStream> {
        val pi = Conexao.app.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
        val id = pi.createSession(params)
        val sessao = pi.openSession(id)
        return sessao to sessao.openWrite("apk", 0, if (tamanho > 0) tamanho else -1)
    }

    private fun confirmar(sessao: PackageInstaller.Session, recebido: String): Resultado {
        val ctx = Conexao.app
        val trava = CountDownLatch(1)
        var resultado = Resultado.erro("A instalação não respondeu em 3 min.")
        val chave = System.identityHashCode(sessao)
        esperando[chave] = { r -> resultado = r; trava.countDown() }
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0)
        val pend = PendingIntent.getBroadcast(ctx, chave, Intent(ctx, Instalacao::class.java).putExtra("chave", chave), flags)
        Acessibilidade.instalarAte = System.currentTimeMillis() + 60_000
        Eventos.anotar("instalar: $recebido, pedindo confirmação ao Android")
        sessao.commit(pend.intentSender)
        sessao.close()
        trava.await(3, TimeUnit.MINUTES)
        esperando.remove(chave)
        Acessibilidade.instalarAte = 0
        return if (resultado.erro) resultado else Resultado.texto("$recebido. ${textoDe(resultado)}")
    }

    private fun textoDe(r: Resultado) = r.conteudo.optJSONObject(0)?.optString("text") ?: ""

    fun aoTerminar(chave: Int, r: Resultado) {
        esperando[chave]?.invoke(r)
    }
}

/** Recebe do PackageInstaller o "precisa de confirmação" e o resultado final. */
class Instalacao : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        val chave = intent.getIntExtra("chave", 0)
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        val msg = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)
        val pacote = intent.getStringExtra(PackageInstaller.EXTRA_PACKAGE_NAME)
        Eventos.anotar("instalação: status=$status pacote=$pacote msg=$msg")
        when (status) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                val confirmar = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT) ?: return
                if (!ctx.packageManager.canRequestPackageInstalls()) {
                    Instalador.aoTerminar(
                        chave,
                        Resultado.erro(
                            "O Android não deixa o Eclipse Mãos instalar apps ainda. Chame `abrir_config {tela: \"instalar_apps\"}`, " +
                                "ligue a chave \"Permitir desta fonte\" e tente de novo.",
                        ),
                    )
                    return
                }
                ctx.startActivity(confirmar.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            PackageInstaller.STATUS_SUCCESS ->
                Instalador.aoTerminar(chave, Resultado.texto("Instalado: $pacote."))
            else -> {
                val porque = when (status) {
                    PackageInstaller.STATUS_FAILURE_ABORTED -> "cancelada na tela de confirmação"
                    PackageInstaller.STATUS_FAILURE_CONFLICT ->
                        "conflito: já existe esse app com OUTRA assinatura (APK de debug sobre o de release, por exemplo)"
                    PackageInstaller.STATUS_FAILURE_INCOMPATIBLE -> "incompatível com esta central"
                    PackageInstaller.STATUS_FAILURE_INVALID -> "APK inválido ou corrompido"
                    PackageInstaller.STATUS_FAILURE_STORAGE -> "sem espaço"
                    else -> "falhou"
                }
                Instalador.aoTerminar(chave, Resultado.erro("Instalação $porque. ${msg ?: ""}".trim()))
            }
        }
    }
}
