// O lado Android do plugin: acha o adaptador, pareia, abre o canal (SPP ou BLE) e
// troca comandos AT/OBD com o ELM327. O módulo OBD em Rust chama estes métodos
// por `run_mobile_plugin`.
//
// Por que o pareamento mora AQUI e não na tela de Bluetooth do Android: naquela
// tela, numa head unit chinesa, ele simplesmente não acontece — o app de
// Bluetooth de fábrica costuma ser de viva-voz e não sabe vincular um aparelho de
// dados. Falando com a pilha direto (`createBond` + resposta de PIN por código),
// o Eclipse não depende dela. É o mesmo caminho do Car Scanner e do Torque.

package com.eclipseos.obdbt

import android.Manifest
import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.os.Build
import android.util.Log
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.concurrent.Executors

@InvokeArg
class ConnectArgs {
    lateinit var address: String

    /** `"spp"` (Bluetooth clássico) ou `"ble"`. */
    var kind: String = "spp"
}

@InvokeArg
class BondArgs {
    lateinit var address: String
}

@InvokeArg
class CommandArgs {
    lateinit var cmd: String
    var timeoutMs: Int = 5000
}

@TauriPlugin(
    permissions = [
        Permission(
            strings = [
                Manifest.permission.BLUETOOTH_CONNECT,
                Manifest.permission.BLUETOOTH_SCAN,
            ],
            alias = "bluetooth",
        ),
        // Android 11 e abaixo exigem localização para QUALQUER busca de Bluetooth,
        // clássica ou BLE — sem ela a lista volta vazia e sem erro. É a ÚNICA de
        // runtime que existe lá, e o Rust pede só ela quando `sdkInt <= 30`: as
        // duas de cima nasceram na API 31 e pedi-las num Android antigo devolve
        // "denied" para sempre. No Android 12+ é o contrário — pede-se as de cima,
        // e o `neverForLocation` da BLUETOOTH_SCAN dispensa esta aqui.
        Permission(
            strings = [Manifest.permission.ACCESS_FINE_LOCATION],
            alias = "location",
        ),
    ],
)
class ObdBtPlugin(private val activity: Activity) : Plugin(activity) {
    // Toda a I/O do canal roda nesta única thread. Dois motivos: o ELM327 é um
    // comando por vez (serializar evita respostas embaralhadas), e nada toca o
    // socket na thread da UI (evita ANR em leituras bloqueantes).
    private val io = Executors.newSingleThreadExecutor()

    // Buscar e parear têm fila PRÓPRIA. Sem isso, um `connect` que bloqueia 30s
    // esperando o carro deixaria a tela de escolha congelada atrás dele — e é
    // justamente quando a conexão não vai que o dono abre aquela tela.
    private val ctrl = Executors.newSingleThreadExecutor()

    private val busca = Busca(activity)
    private var link: Link? = null

    @Volatile
    private var pareando = false

    /**
     * Enquanto uma busca ou um pareamento corre, ninguém conecta.
     *
     * Derivado, e não um booleano guardado: um `startScan` sem o `stopScan`
     * correspondente (a tela fechou, o app caiu) deixaria a flag presa em `true` e
     * o carro nunca mais conectaria sozinho. Perguntar à busca não tem esse risco.
     */
    private val ocupadoNoControle: Boolean
        get() = pareando || busca.ativa

    @Command
    fun info(invoke: Invoke) {
        val adapter = BluetoothAdapter.getDefaultAdapter()
        val ret = JSObject()
        ret.put("sdkInt", Build.VERSION.SDK_INT)
        ret.put("existe", adapter != null)
        // `isEnabled` exige BLUETOOTH_CONNECT no Android 12+ e joga
        // SecurityException sem ela. Isto aqui é a PRIMEIRA coisa que o lado
        // Rust chama — justamente para decidir QUAIS permissões pedir — então
        // estourar aqui deixaria o app sem saber nem em que Android está.
        ret.put(
            "ligado",
            try {
                adapter?.isEnabled ?: false
            } catch (e: SecurityException) {
                Log.i(TAG, "ainda sem permissão para ler o estado do rádio: ${e.message}")
                false
            },
        )
        invoke.resolve(ret)
    }

    @Command
    fun listBonded(invoke: Invoke) {
        ctrl.execute {
            comAdaptador(invoke) { adapter ->
                val arr = JSArray()
                for (device in adapter.bondedDevices.orEmpty()) {
                    Log.i(TAG, "pareado: \"${device.name ?: ""}\" (${device.address})")
                    arr.put(jsAchado(achadoDe(device)))
                }
                val ret = JSObject()
                ret.put("devices", arr)
                invoke.resolve(ret)
            }
        }
    }

    @Command
    fun startScan(invoke: Invoke) {
        ctrl.execute {
            comAdaptador(invoke) { adapter ->
                busca.iniciar(adapter)
                invoke.resolve()
            }
        }
    }

    @Command
    fun scanResults(invoke: Invoke) {
        // Fora do `ctrl` de propósito: ler o mapa é barato e não pode ficar atrás
        // de um pareamento de 20s, senão a lista congela justo enquanto pareia.
        val arr = JSArray()
        for (a in busca.resultados()) arr.put(jsAchado(a))
        val ret = JSObject()
        ret.put("devices", arr)
        ret.put("scanning", busca.ativa)
        invoke.resolve(ret)
    }

    @Command
    fun stopScan(invoke: Invoke) {
        ctrl.execute {
            comAdaptador(invoke) { adapter ->
                busca.parar(adapter)
                invoke.resolve()
            }
        }
    }

    @Command
    fun bond(invoke: Invoke) {
        val args = invoke.parseArgs(BondArgs::class.java)
        ctrl.execute {
            comAdaptador(invoke) { adapter ->
                pareando = true
                try {
                    val device = adapter.getRemoteDevice(args.address)
                    if (Pareamento.parear(activity, adapter, device)) {
                        invoke.resolve()
                    } else {
                        invoke.reject("não consegui parear com ${args.address}")
                    }
                } finally {
                    pareando = false
                }
            }
        }
    }

    @Command
    fun connect(invoke: Invoke) {
        val args = invoke.parseArgs(ConnectArgs::class.java)
        if (ocupadoNoControle) {
            // Recusar agora é melhor que competir: o rádio está buscando ou
            // pareando, e o módulo OBD sabe esperar (o supervisor reconecta com
            // backoff). Competir deixaria as duas coisas ruins.
            invoke.reject("o rádio está ocupado com a busca/pareamento")
            return
        }
        io.execute {
            comAdaptador(invoke) { adapter ->
                fecharCalado()
                val device = adapter.getRemoteDevice(args.address)
                Log.i(TAG, "conectando em ${args.address} via ${args.kind}")
                link = if (args.kind == TIPO_BLE) {
                    BleLink.abrir(activity, device, ESPERA_BLE_MS)
                } else {
                    SppLink.abrir(adapter, device)
                }
                Log.i(TAG, "conectado em ${args.address}")
                invoke.resolve()
            }
        }
    }

    @Command
    fun command(invoke: Invoke) {
        val args = invoke.parseArgs(CommandArgs::class.java)
        io.execute {
            val canal = link
            if (canal == null) {
                invoke.reject("adaptador não conectado")
                return@execute
            }
            try {
                val resposta = canal.send(args.cmd, args.timeoutMs)
                Log.d(TAG, "${args.cmd} -> ${resposta.replace("\r", "|")}")
                val ret = JSObject()
                ret.put("response", resposta)
                invoke.resolve(ret)
            } catch (e: ObdBtFalha) {
                Log.w(TAG, "${args.cmd}: ${e.message}")
                invoke.reject(e.message ?: "o adaptador não respondeu")
            } catch (e: Exception) {
                invoke.reject("falha ao falar com o adaptador: ${e.message}")
            }
        }
    }

    @Command
    fun disconnect(invoke: Invoke) {
        io.execute {
            fecharCalado()
            invoke.resolve()
        }
    }

    /**
     * Roda o corpo com o adaptador em mãos, virando qualquer exceção em `reject`.
     *
     * Existe porque as cinco checagens de sempre (tem rádio? está ligado? tenho
     * permissão?) apareciam copiadas em cada comando, e cada cópia era uma chance
     * de um erro subir como pânico em vez de mensagem na tela.
     */
    private fun comAdaptador(invoke: Invoke, corpo: (BluetoothAdapter) -> Unit) {
        try {
            val adapter = BluetoothAdapter.getDefaultAdapter()
            if (adapter == null) {
                invoke.reject("aparelho sem Bluetooth")
                return
            }
            if (!adapter.isEnabled) {
                invoke.reject("Bluetooth desligado")
                return
            }
            corpo(adapter)
        } catch (e: SecurityException) {
            invoke.reject("sem permissão de Bluetooth: ${e.message}")
        } catch (e: ObdBtFalha) {
            Log.w(TAG, "falha: ${e.message}")
            invoke.reject(e.message ?: "falha no adaptador")
        } catch (e: Exception) {
            Log.w(TAG, "erro inesperado: ${e.message}")
            invoke.reject(e.message ?: e.javaClass.simpleName)
        }
    }

    private fun achadoDe(device: BluetoothDevice): Achado {
        val tipo = try {
            if (device.type == BluetoothDevice.DEVICE_TYPE_LE) TIPO_BLE else TIPO_SPP
        } catch (e: SecurityException) {
            TIPO_SPP
        }
        return Achado(device.name ?: "", device.address, tipo, pareado = true, rssi = null)
    }

    private fun jsAchado(a: Achado): JSObject {
        val obj = JSObject()
        obj.put("name", a.nome)
        obj.put("address", a.mac)
        obj.put("kind", a.tipo)
        obj.put("bonded", a.pareado)
        if (a.rssi != null) obj.put("rssi", a.rssi)
        return obj
    }

    /**
     * Fecha a conexão anterior antes de abrir outra.
     *
     * A espera depois do `close` não é frescura: a pilha de Bluetooth do Android
     * não solta o canal RFCOMM na mesma instância em que o socket é fechado, e
     * reconectar no mesmo adaptador imediatamente falha com "read failed, socket
     * might closed or timeout". Como quem chama isto é o `connect` depois de uma
     * queda, é exatamente o caminho do reinício do módulo OBD.
     */
    private fun fecharCalado() {
        val anterior = link ?: return
        anterior.close()
        link = null
        Thread.sleep(ESPERA_APOS_FECHAR_MS)
    }

    private companion object {
        /** Ver `fecharCalado`. */
        const val ESPERA_APOS_FECHAR_MS = 500L

        /** Conectar por GATT é rápido quando dá certo; 15s já é desistência. */
        const val ESPERA_BLE_MS = 15_000L
    }
}
