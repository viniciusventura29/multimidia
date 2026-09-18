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
import java.util.UUID

internal class SppLink private constructor(private val socket: BluetoothSocket) : Link {
    private val input = socket.inputStream
    private val output = socket.outputStream

    companion object {
        /** UUID padrão do Serial Port Profile — é o que o ELM327 fala. */
        private val SPP_UUID: UUID = UUID.fromString("00001101-0000-1000-8000-00805F9B34FB")

        fun abrir(adapter: BluetoothAdapter, device: BluetoothDevice): SppLink {
            // Descoberta ativa deixa o handshake do RFCOMM lento e instável — mas
            // cancelar é só otimização, e exige BLUETOOTH_SCAN. Se o usuário negou
            // SCAN, conecta mesmo assim.
            try {
                adapter.cancelDiscovery()
            } catch (e: SecurityException) {
                Log.w(TAG, "sem BLUETOOTH_SCAN para cancelDiscovery; seguindo sem cancelar")
            }

            val socket = try {
                val spp = device.createRfcommSocketToServiceRecord(SPP_UUID)
                spp.connect() // bloqueia até conectar ou estourar
                spp
            } catch (e: Exception) {
                // Clones de ELM327 às vezes não anunciam o service record direito;
                // o caminho clássico é cair para o canal RFCOMM 1 por reflexão (o
                // mesmo que os apps de scanner fazem).
                Log.w(TAG, "SPP por service record falhou (${e.message}); tentando canal 1")
                val m = device.javaClass.getMethod("createRfcommSocket", Int::class.javaPrimitiveType)
                val canal1 = m.invoke(device, 1) as BluetoothSocket
                canal1.connect()
                canal1
            }
            return SppLink(socket)
        }
    }

    override fun send(cmd: String, timeoutMs: Int): String {
        output.write((cmd + "\r").toByteArray(Charsets.US_ASCII))
        output.flush()

        val coletor = Coletor()
        val limite = System.currentTimeMillis() + timeoutMs
        val buf = ByteArray(64)
        var achouPrompt = false
        while (System.currentTimeMillis() < limite) {
            if (input.available() > 0) {
                val n = input.read(buf)
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
