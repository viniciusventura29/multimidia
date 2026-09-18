// O canal de bytes com o adaptador, e o que é igual nos dois transportes.
//
// O ELM327 termina TODA resposta com o prompt '>'. Quem carrega os bytes muda
// (socket RFCOMM no clássico, notificação GATT no BLE); esperar o '>' e montar a
// resposta não muda. É esse pedaço que mora aqui.

package com.eclipseos.obdbt

import android.content.BroadcastReceiver
import android.content.Context
import android.content.IntentFilter
import android.os.Build

/** Filtre o logcat por esta tag para ver a conversa com o adaptador:
 *    adb logcat -s EclipseObdBt */
internal const val TAG = "EclipseObdBt"

/**
 * Um canal aberto com o adaptador.
 *
 * `send` é **bloqueante e um por vez** de propósito: o ELM327 é um comando por
 * vez, e duas perguntas ao mesmo tempo voltam embaralhadas. Quem garante a fila
 * é o executor único do plugin.
 */
internal interface Link {
    /** Manda o comando (sem `\r`) e devolve a resposta crua até o prompt. */
    fun send(cmd: String, timeoutMs: Int): String

    fun close()
}

/**
 * Acumula bytes até o prompt `>`.
 *
 * O prompt em si não entra na resposta — ele é pontuação do adaptador, não dado
 * do carro.
 */
internal class Coletor {
    private val sb = StringBuilder()

    /** Empurra `n` bytes; devolve `true` quando o prompt chegou. */
    fun push(buf: ByteArray, n: Int): Boolean {
        for (i in 0 until n) {
            val c = buf[i].toInt().toChar()
            if (c == '>') return true
            sb.append(c)
        }
        return false
    }

    fun vazio(): Boolean = sb.isEmpty()

    fun texto(): String = sb.toString().trim()
}

/**
 * Silêncio total é falha; resposta truncada não é.
 *
 * Um clone que não manda o prompt depois de um `ATZ` é comum, e o handshake
 * sobrevive a isso. Já zero byte dentro do prazo significa adaptador mudo — e aí
 * o módulo OBD precisa saber, para o supervisor reconectar.
 */
internal fun resposta(coletor: Coletor, cmd: String, timeoutMs: Int, achouPrompt: Boolean): String {
    if (!achouPrompt && coletor.vazio()) {
        throw ObdBtFalha("adaptador não respondeu $cmd em ${timeoutMs}ms")
    }
    return coletor.texto()
}

/** Falha esperada do adaptador — vira `reject` na ponte, não pânico. */
internal class ObdBtFalha(mensagem: String) : Exception(mensagem)


/**
 * Registra um receptor de broadcast do sistema.
 *
 * A partir do Android 14 registrar sem dizer se o receptor é exportado joga
 * exceção. Os broadcasts de Bluetooth são protegidos (só o sistema os manda), então
 * `NOT_EXPORTED` é o certo — e `ContextCompat` fica de fora para o plugin não
 * depender de qual versão do androidx a central trouxe.
 */
internal fun registrarReceptor(
    context: Context,
    receptor: BroadcastReceiver,
    filtro: IntentFilter,
) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        context.registerReceiver(receptor, filtro, Context.RECEIVER_NOT_EXPORTED)
    } else {
        context.registerReceiver(receptor, filtro)
    }
}
