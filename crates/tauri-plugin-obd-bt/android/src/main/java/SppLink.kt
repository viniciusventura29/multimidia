// Bluetooth clássico: o socket RFCOMM/SPP com o ELM327.
//
// Era todo o plugin até existir BLE; agora é uma das duas implementações de
// [Link]. O miolo (service record, queda para o canal 1 por reflexão, ler até o
// prompt) é o mesmo que já rodava no carro.

package com.eclipseos.obdbt

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothSocket
import android.util.Log
import java.io.IOException
import java.util.UUID

internal class SppLink private constructor(private val socket: BluetoothSocket) : Link {
    private val input = socket.inputStream
    private val output = socket.outputStream

    companion object {
        /** UUID padrão do Serial Port Profile — é o que o ELM327 fala. */
        private val SPP_UUID: UUID = UUID.fromString("00001101-0000-1000-8000-00805F9B34FB")

        /**
         * Entre fechar um socket que falhou e tentar o próximo.
         *
         * Meio segundo é o que a pilha de Bluetooth do Android costuma levar para
         * soltar o canal de verdade. Não é folga de estilo: sem ela a segunda
         * tentativa falha pela sujeira da primeira, e não por si.
         */
        private const val ESPERA_ENTRE_TENTATIVAS_MS = 500L

        fun abrir(adapter: BluetoothAdapter, device: BluetoothDevice): SppLink {
            // Descoberta ativa deixa o handshake do RFCOMM lento e instável — mas
            // cancelar é só otimização, e exige BLUETOOTH_SCAN. Se o usuário negou
            // SCAN, conecta mesmo assim.
            try {
                adapter.cancelDiscovery()
            } catch (e: SecurityException) {
                Log.w(TAG, "sem BLUETOOTH_SCAN para cancelDiscovery; seguindo sem cancelar")
            }

            return SppLink(conectar(device))
        }

        /**
         * Abre o socket, com as duas tentativas e — o que faltava — fechando o
         * que não deu certo.
         *
         * Um socket RFCOMM que falhou no `connect` e não foi fechado continua
         * segurando recurso na pilha de Bluetooth do Android, e a tentativa
         * seguinte contra o MESMO aparelho falha com "read failed, socket might
         * closed or timeout, read ret: -1" — que foi exatamente o que o diário
         * de bordo trouxe do carro, duas vezes seguidas antes de conectar na
         * terceira. Cada tentativa perdida custava um reinício do módulo.
         */
        private fun conectar(device: BluetoothDevice): BluetoothSocket {
            val porServiceRecord = device.createRfcommSocketToServiceRecord(SPP_UUID)
            try {
                porServiceRecord.connect() // bloqueia até conectar ou estourar
                return porServiceRecord
            } catch (e: Exception) {
                // Clones de ELM327 às vezes não anunciam o service record direito;
                // o caminho clássico é cair para o canal RFCOMM 1 por reflexão (o
                // mesmo que os apps de scanner fazem).
                Log.w(TAG, "SPP por service record falhou (${e.message}); tentando canal 1")
                fecharCalado(porServiceRecord)
            }

            // Respirar entre as duas: a pilha do Android não libera o canal na
            // mesma instância em que o socket é fechado, e emendar o segundo
            // `connect` no primeiro faz a queda para o canal 1 falhar por um
            // motivo que não é o dela.
            Thread.sleep(ESPERA_ENTRE_TENTATIVAS_MS)

            val m = device.javaClass.getMethod("createRfcommSocket", Int::class.javaPrimitiveType)
            val canal1 = m.invoke(device, 1) as BluetoothSocket
            try {
                canal1.connect()
                return canal1
            } catch (e: Exception) {
                // Também aqui: sem fechar, o próximo `connect` herda a sujeira
                // desta tentativa e falha por ela, não por si.
                fecharCalado(canal1)
                throw e
            }
        }

        private fun fecharCalado(socket: BluetoothSocket) {
            try {
                socket.close()
            } catch (e: Exception) {
                Log.w(TAG, "não consegui fechar o socket que falhou: ${e.message}")
            }
        }
    }

    override fun send(cmd: String, timeoutMs: Int): String {
        // Escrever num socket morto dá erro na hora — e é a detecção mais
        // barata que existe, antes de gastar o prazo esperando resposta.
        try {
            output.write((cmd + "\r").toByteArray(Charsets.US_ASCII))
            output.flush()
        } catch (e: IOException) {
            throw ObdBtLinkMorto("escrita falhou: ${e.message}")
        }

        val coletor = Coletor()
        val limite = System.currentTimeMillis() + timeoutMs
        val buf = ByteArray(64)
        var achouPrompt = false
        while (System.currentTimeMillis() < limite) {
            // `available()` de um socket MORTO devolve zero, igualzinho a um
            // socket vivo e quieto. Sem olhar `isConnected` os dois casos ficam
            // indistinguíveis, e um canal caído viraria "adaptador não
            // respondeu" — que o poller trata como transitório e repete, PID
            // por PID, prazo cheio cada. Era essa a queda de 32 em 32 segundos.
            if (!socket.isConnected) {
                throw ObdBtLinkMorto("o socket RFCOMM caiu durante $cmd")
            }
            val disponivel =
                try {
                    input.available()
                } catch (e: IOException) {
                    throw ObdBtLinkMorto("leitura falhou: ${e.message}")
                }
            if (disponivel > 0) {
                val n =
                    try {
                        input.read(buf)
                    } catch (e: IOException) {
                        throw ObdBtLinkMorto("leitura falhou: ${e.message}")
                    }
                // Fim de stream. Num socket de rede isto é o outro lado
                // fechando; aqui significa adaptador desligado ou fora de
                // alcance, e nenhuma espera adicional vai trazer bytes.
                if (n < 0) {
                    throw ObdBtLinkMorto("o adaptador fechou o canal durante $cmd")
                }
                if (n > 0 && coletor.push(buf, n)) {
                    achouPrompt = true
                    break
                }
            } else {
                Thread.sleep(10)
            }
        }
        return resposta(coletor, cmd, timeoutMs, achouPrompt)
    }

    override fun close() {
        try { input.close() } catch (_: Exception) {}
        try { output.close() } catch (_: Exception) {}
        try { socket.close() } catch (_: Exception) {}
    }
}
