package com.eclipseos.maos

import android.app.Application
import android.content.ComponentCallbacks2
import android.content.Context
import android.util.Log
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * O que aconteceu com o Mãos, gravado em disco para sobreviver à morte do
 * processo.
 *
 * Existe por um motivo só: no Android 10, quando o processo de um serviço de
 * acessibilidade morre (erro nosso OU a central matando o app), o sistema o
 * marca como "não está funcionando" e não religa sozinho — e do lado de fora
 * isso é só "o carro sumiu". Sem um registro que sobreviva à morte, não dá para
 * saber se foi exceção (aparece a pilha) ou morte de fora (o processo
 * simplesmente recomeça, sem pilha nenhuma antes).
 */
object Eventos {
    private const val MAX_LINHAS = 300
    private val formato = SimpleDateFormat("dd/MM HH:mm:ss", Locale.US)
    private var arquivo: File? = null

    fun iniciar(ctx: Context) {
        arquivo = File(ctx.filesDir, "eventos.log")
    }

    @Synchronized
    fun anotar(msg: String) {
        Log.i("maos", msg)
        val f = arquivo ?: return
        try {
            f.appendText("${formato.format(Date())} $msg\n")
            if (f.length() > 64 * 1024) {
                val ultimas = f.readLines().takeLast(MAX_LINHAS)
                f.writeText(ultimas.joinToString("\n", postfix = "\n"))
            }
        } catch (_: Exception) {
        }
    }

    fun ultimas(n: Int): String {
        val f = arquivo ?: return "sem registro"
        if (!f.exists()) return "sem registro"
        return f.readLines().takeLast(n).joinToString("\n")
    }
}

/** Liga o registro antes de qualquer outra coisa do processo. */
class MaosApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Eventos.iniciar(this)
        // A linha que separa uma vida do processo da outra: se a anterior não
        // terminou com "morreu: …", ele foi morto de fora.
        Eventos.anotar("── processo iniciou")
        val anterior = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, e ->
            Eventos.anotar("morreu: exceção na thread ${thread.name}: ${Log.getStackTraceString(e).take(3000)}")
            anterior?.uncaughtException(thread, e)
        }
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        if (level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW) Eventos.anotar("memória baixa (trim $level)")
    }

    override fun onLowMemory() {
        super.onLowMemory()
        Eventos.anotar("memória baixa (onLowMemory)")
    }
}
