// O lado Android do plugin: abre um socket Bluetooth clássico (SPP/RFCOMM) com o
// ELM327 e troca comandos AT/OBD com ele. O módulo OBD em Rust chama estes
// métodos por `run_mobile_plugin`.

package com.eclipseos.obdbt

import android.Manifest
import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothSocket
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.InputStream
import java.io.OutputStream
import java.util.UUID
import java.util.concurrent.Executors

@InvokeArg
class ConnectArgs {
    lateinit var address: String
}

@InvokeArg
class CommandArgs {
    lateinit var cmd: String
    var timeoutMs: Int = 5000
}

@TauriPlugin(
    permissions = [
        Permission(strings = [Manifest.permission.BLUETOOTH_CONNECT], alias = "bluetooth")
    ]
)
class ObdBtPlugin(private val activity: Activity) : Plugin(activity) {
    companion object {
        // UUID padrão do Serial Port Profile (SPP) — é o que o ELM327 fala.
        private val SPP_UUID: UUID = UUID.fromString("00001101-0000-1000-8000-00805F9B34FB")
    }

    // Toda a I/O do socket roda nesta única thread. Dois motivos: o ELM327 é um
    // comando por vez (serializar evita respostas embaralhadas), e nada toca o
    // socket na thread da UI (evita ANR em leituras bloqueantes).
    private val io = Executors.newSingleThreadExecutor()

    private var socket: BluetoothSocket? = null
    private var input: InputStream? = null
    private var output: OutputStream? = null

    @Command
    fun listBonded(invoke: Invoke) {
        io.execute {
            try {
                val adapter = BluetoothAdapter.getDefaultAdapter()
                if (adapter == null) {
                    invoke.reject("aparelho sem Bluetooth")
                    return@execute
                }
                if (!adapter.isEnabled) {
                    invoke.reject("Bluetooth desligado")
                    return@execute
                }
                val arr = JSArray()
                for (device in adapter.bondedDevices) {
                    val obj = JSObject()
                    obj.put("name", device.name ?: "")
                    obj.put("address", device.address)
                    arr.put(obj)
                }
                val ret = JSObject()
                ret.put("devices", arr)
                invoke.resolve(ret)
            } catch (e: SecurityException) {
                invoke.reject("sem permissão de Bluetooth: ${e.message}")
            } catch (e: Exception) {
                invoke.reject("falha ao listar pareados: ${e.message}")
            }
        }
    }

    @Command
    fun connect(invoke: Invoke) {
        val args = invoke.parseArgs(ConnectArgs::class.java)
        io.execute {
            try {
                closeQuietly()
                val adapter = BluetoothAdapter.getDefaultAdapter()
                if (adapter == null) {
                    invoke.reject("aparelho sem Bluetooth")
                    return@execute
                }
                val device = adapter.getRemoteDevice(args.address)
                // Descoberta ativa deixa o handshake do RFCOMM lento e instável.
                adapter.cancelDiscovery()
                val s = device.createRfcommSocketToServiceRecord(SPP_UUID)
                s.connect() // bloqueia até conectar ou estourar
                socket = s
                input = s.inputStream
                output = s.outputStream
                invoke.resolve()
            } catch (e: SecurityException) {
                closeQuietly()
                invoke.reject("sem permissão de Bluetooth: ${e.message}")
            } catch (e: Exception) {
                closeQuietly()
                invoke.reject("não conectou no adaptador: ${e.message}")
            }
        }
    }

    @Command
    fun command(invoke: Invoke) {
        val args = invoke.parseArgs(CommandArgs::class.java)
        io.execute {
            val out = output
            val inp = input
            if (out == null || inp == null) {
                invoke.reject("adaptador não conectado")
                return@execute
            }
            try {
                out.write((args.cmd + "\r").toByteArray(Charsets.US_ASCII))
                out.flush()

                // O ELM327 termina toda resposta com o prompt '>'. Lê até vê-lo
                // ou até o tempo acabar.
                val sb = StringBuilder()
                val deadline = System.currentTimeMillis() + args.timeoutMs
                val buf = ByteArray(64)
                var achouPrompt = false
                while (System.currentTimeMillis() < deadline) {
                    if (inp.available() > 0) {
                        val n = inp.read(buf)
                        if (n > 0) {
                            for (i in 0 until n) {
                                val c = buf[i].toInt().toChar()
                                if (c == '>') {
                                    achouPrompt = true
                                    break
                                }
                                sb.append(c)
                            }
                        }
                        if (achouPrompt) break
                    } else {
                        Thread.sleep(10)
                    }
                }

                if (!achouPrompt && sb.isEmpty()) {
                    invoke.reject("adaptador não respondeu (timeout)")
                    return@execute
                }
                val ret = JSObject()
                ret.put("response", sb.toString().trim())
                invoke.resolve(ret)
            } catch (e: Exception) {
                invoke.reject("falha ao falar com o adaptador: ${e.message}")
            }
        }
    }

    @Command
    fun disconnect(invoke: Invoke) {
        io.execute {
            closeQuietly()
            invoke.resolve()
        }
    }

    private fun closeQuietly() {
        try { input?.close() } catch (_: Exception) {}
        try { output?.close() } catch (_: Exception) {}
        try { socket?.close() } catch (_: Exception) {}
        input = null
        output = null
        socket = null
    }
}
