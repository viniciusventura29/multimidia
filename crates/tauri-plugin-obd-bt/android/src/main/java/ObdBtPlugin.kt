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
import android.provider.Settings
import android.util.Log
import org.json.JSONArray
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

@InvokeArg
class AppRemoteTocarArgs {
    /** Client ID e redirect vêm do Rust: o plugin não conhece credencial. */
    lateinit var clientId: String
    lateinit var redirectUri: String
    var uri: String? = null
    var contexto: String? = null
    /** Posição da faixa dentro do contexto. `-1` = do começo. */
    var indice: Int = -1
}

@InvokeArg
class AppRemoteComandoArgs {
    lateinit var clientId: String
    lateinit var redirectUri: String
    lateinit var acao: String
    var valor: Long = 0
}

@InvokeArg
class ComandoSessaoArgs {
    /** `tocar`, `pausar`, `alternar`, `proxima`, `anterior`, `saltar`. */
    lateinit var acao: String

    /** Só o `saltar` usa: a posição em milissegundos. */
    var valor: Long = 0
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

    /**
     * A posição vinda do Android, sem passar pela WebView.
     *
     * Ver `Localizacao.kt`: o `navigator.geolocation` nunca entregou nada nesta
     * central porque o pedido não chega ao Android. Aqui o Rust puxa, de tempos
     * em tempos, a última posição que o sistema conhece.
     *
     * Não usa `io.execute`: só lê um campo já preenchido pelo ouvinte, e o
     * registro dos provedores acontece na primeira chamada.
     */
    @Command
    fun ultimaPosicao(invoke: Invoke) {
        try {
            val json = Localizacao.ultima(activity)

            // Sem posição, e só então, pede o diálogo de precisão. A `Activity`
            // é necessária para abri-lo, e ela existe aqui e não dentro do
            // `Localizacao`, que só conhece `Context`.
            //
            // Depois de ler, e não antes: quando já há posição não há nada a
            // consertar, e o diálogo seria interrupção gratuita em cima de quem
            // está dirigindo. `pedirPrecisao` também se protege sozinho — uma
            // vez por processo, e calado se o ajuste já estiver bom.
            if (!json.optBoolean("tem", false)) {
                Localizacao.pedirPrecisao(activity)
            }

            val ret = JSObject()
            ret.put("json", json.toString())
            invoke.resolve(ret)
        } catch (e: Exception) {
            invoke.reject(e.message ?: e.javaClass.simpleName)
        }
    }

    /**
     * Sonda temporária: o Spotify deixa o Eclipse navegar na biblioteca dele?
     *
     * Não tem nada a ver com Bluetooth e está aqui por economia — ver
     * `SondaMedia.kt`. Sai quando a resposta chegar.
     */
    @Command
    fun sondarMedia(invoke: Invoke) {
        io.execute {
            try {
                val json = SondaMedia.sondar(activity)
                val ret = JSObject()
                ret.put("json", json.toString())
                invoke.resolve(ret)
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.javaClass.simpleName)
            }
        }
    }

    /**
     * O que o app do Spotify DESTE aparelho está tocando.
     *
     * Direto da `MediaSession` local: sem rede, sem nuvem, e com a capa já
     * pronta em bitmap. Ver `SessaoMedia.kt` para por que isto existe.
     *
     * Não usa `io.execute`: são chamadas de binder, que voltam em
     * microssegundos. Mandar para outra thread custaria mais que a leitura.
     */
    @Command
    fun sessaoMediaEstado(invoke: Invoke) {
        try {
            val ret = JSObject()
            ret.put("json", SessaoMedia.estado(activity).toString())
            invoke.resolve(ret)
        } catch (e: Exception) {
            invoke.reject(e.message ?: e.javaClass.simpleName)
        }
    }

    /**
     * Um toque de transporte na sessão local.
     *
     * `atendeu: false` não é erro — é o Rust sendo avisado de que precisa
     * repetir o comando pela Web API.
     */
    @Command
    fun sessaoMediaComando(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(ComandoSessaoArgs::class.java)
            val ret = JSObject()
            ret.put("atendeu", SessaoMedia.comando(activity, args.acao, args.valor))
            invoke.resolve(ret)
        } catch (e: Exception) {
            invoke.reject(e.message ?: e.javaClass.simpleName)
        }
    }

    /**
     * O nome deste aparelho, como o Android o conhece.
     *
     * Serve para uma pergunta só, e ela é surpreendentemente difícil sem isto:
     * **qual dos dispositivos do Spotify Connect é ESTA central?**
     *
     * O app do Spotify se anuncia no Connect com o nome do aparelho. Sem esse
     * nome, a lista traz "o celular dele" e "a central" as duas como
     * `Smartphone`, indistinguíveis — e mandar o som para a errada é pior que
     * não mandar.
     *
     * `DEVICE_NAME` é o nome que o dono deu em Configurações; `Build.MODEL` é
     * o de fábrica, e é o que o Spotify usa quando não há o outro.
     */
    @Command
    fun nomeDoAparelho(invoke: Invoke) {
        try {
            // TODOS os nomes, não o primeiro que existir.
            //
            // A versão anterior fazia `DEVICE_NAME ?: Build.MODEL` — só caía no
            // modelo se o nome de configuração faltasse. No carro do dono os
            // dois existem E SÃO DIFERENTES: o Android diz "K706" (o nome que
            // ele deu nas configurações) e o Spotify se anuncia como
            // "HT-9960CA" (o modelo de fábrica).
            //
            // Resultado: o app do Spotify da central ESTAVA na lista, e era
            // rejeitado por não casar com o único nome que eu mandava. A tela
            // pedia para abrir o Spotify que já estava aberto.
            val nomes = mutableListOf<String>()
            try {
                Settings.Global.getString(activity.contentResolver, Settings.Global.DEVICE_NAME)
                    ?.let { nomes.add(it) }
            } catch (e: Exception) {
                Log.w(TAG, "sem DEVICE_NAME: ${e.message}")
            }
            Build.MODEL?.let { nomes.add(it) }
            Build.DEVICE?.let { nomes.add(it) }
            Build.PRODUCT?.let { nomes.add(it) }

            val ret = JSObject()
            val limpos = nomes.map { it.trim() }.filter { it.isNotEmpty() }.distinct()
            ret.put("nomes", JSONArray(limpos))
            // Mantido para quem só quer um: é o mais "humano" dos quatro.
            ret.put("nome", limpos.firstOrNull() ?: "")
            invoke.resolve(ret)
        } catch (e: Exception) {
            invoke.reject(e.message ?: e.javaClass.simpleName)
        }
    }

    /**
     * Manda o app do Spotify DESTA central tocar.
     *
     * Ver `AppRemoteSpotify.kt` para por que isto existe e por que a Web API
     * não dava conta.
     *
     * Usa `io.execute`: conectar pode ter de INICIAR o app do Spotify, e isso
     * demora — segurar a thread que chamou por doze segundos travaria a ponte.
     */
    @Command
    fun appRemoteTocar(invoke: Invoke) {
        io.execute {
            try {
                val a = invoke.parseArgs(AppRemoteTocarArgs::class.java)
                val r = AppRemoteSpotify.tocar(
                    activity,
                    a.clientId,
                    a.redirectUri,
                    a.uri,
                    a.contexto,
                    a.indice,
                )
                val ret = JSObject()
                ret.put("json", r.toString())
                invoke.resolve(ret)
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.javaClass.simpleName)
            }
        }
    }

    /** Um toque de transporte no app do Spotify da central. */
    @Command
    fun appRemoteComando(invoke: Invoke) {
        io.execute {
            try {
                val a = invoke.parseArgs(AppRemoteComandoArgs::class.java)
                val r = AppRemoteSpotify.comando(
                    activity,
                    a.clientId,
                    a.redirectUri,
                    a.acao,
                    a.valor,
                )
                val ret = JSObject()
                ret.put("json", r.toString())
                invoke.resolve(ret)
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.javaClass.simpleName)
            }
        }
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
            } catch (e: ObdBtLinkMorto) {
                // O canal morreu: fechar AQUI, e não esperar o próximo
                // `connect`. Um socket RFCOMM que continua aberto segue
                // segurando o canal na pilha do Android, e a reconexão contra o
                // mesmo aparelho falha com "read failed, socket might closed or
                // timeout" — por causa do cadáver, não por si.
                //
                // Zerar o `link` também faz os comandos que já estavam na fila
                // falharem na hora com "adaptador não conectado", em vez de
                // cada um esperar o prazo cheio conversando com um morto.
                Log.w(TAG, "canal caiu em ${args.cmd}: ${e.message}")
                fecharCalado()
                invoke.reject(e.message ?: "o canal com o adaptador caiu")
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
