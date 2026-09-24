// A posição vinda do Android, e não da WebView.
//
// O `navigator.geolocation` nunca entregou nada nesta central — nem por
// satélite nem por rede, com timeout idêntico nos dois. Esse empate é a
// assinatura de um pedido que NÃO CHEGA ao sistema: a WebView do Android só
// libera geolocalização para a página se o app hospedeiro responder o
// `onGeolocationPermissionsShowPrompt`, e o Tauri não responde.
//
// Trocar a WebView pelo `LocationManager` resolveu esse pedaço: o diário passou
// a dizer "ainda sem posição" em vez de "timeout", o que prova que a permissão
// existe e que os provedores registraram sem erro. Mas posição não veio.
//
// O que falta agora não é caminho, é DIAGNÓSTICO. "Ainda sem posição" não
// distingue localização desligada nas configurações, de antena não plugada, de
// céu ruim — e cada uma dessas tem um dono diferente. Então este arquivo passa
// a medir, e o número que decide é a contagem de satélites: zero visíveis
// sempre = antena ou desligado; visíveis sem nenhum usado = sinal fraco.
//
// ⚠️ Mora no plugin de Bluetooth porque é aqui que o Eclipse já fala com o
// Android, e porque este plugin já pede ACCESS_FINE_LOCATION para varrer
// adaptador no Android ≤ 11.

package com.eclipseos.obdbt

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.location.GnssStatus
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.content.ContextCompat
import com.google.android.gms.common.api.ResolvableApiException
import com.google.android.gms.location.FusedLocationProviderClient
import com.google.android.gms.location.LocationCallback
import com.google.android.gms.location.LocationRequest
import com.google.android.gms.location.LocationResult
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.LocationSettingsRequest
import com.google.android.gms.location.Priority
import org.json.JSONArray
import org.json.JSONObject

/** De quanto em quanto tempo o Android pode mandar posição nova. */
private const val INTERVALO_MS = 1_000L

/** Distância mínima para uma posição nova valer. Zero: quem filtra parada é o
 *  `FiltroDeParada` do Rust, e dois filtros discordando é pior que um. */
private const val DISTANCIA_MIN_M = 0f

/** O corte clássico do Android para "a posição velha não vale mais". */
private const val VELHA_DEMAIS_MS = 120_000L

/** Código do `startResolutionForResult`. Não lemos a resposta — o próprio
 *  sistema liga o ajuste, e o provedor fundido passa a entregar sozinho. */
private const val CODIGO_PRECISAO = 7311

/**
 * Quanto esperar sem NENHUMA posição antes de registrar tudo de novo.
 *
 * Existe por causa de um caso real: o dono abriu o app, aceitou o diálogo de
 * precisão, e o mapa continuou vazio — só funcionou depois de fechar e abrir.
 *
 * O motivo é que o registro é feito UMA vez, e naquele momento o provedor de
 * rede ainda estava desligado. Ligar o ajuste depois não faz o Android
 * reentregar nada a quem já tinha se registrado contra um provedor morto: o
 * pedido antigo continua de pé, apontando para o nada.
 *
 * Meio minuto é longo o bastante para não atrapalhar uma primeira fixação
 * legítima (um GPS frio leva de 30 a 60 s), e curto o bastante para o dono não
 * precisar fechar o app.
 */
private const val PACIENCIA_ANTES_DE_RELIGAR_MS = 30_000L

/**
 * Teto da espera entre religadas.
 *
 * A espera DOBRA a cada tentativa, e isto é um conserto de um defeito que eu
 * mesmo criei: religar de 30 em 30 segundos, para sempre, derruba o pedido de
 * localização antes de ele ter chance de responder.
 *
 * Localização por rede não é instantânea — ela varre Wi-Fi e consulta o Google
 * para transformar isso em coordenada. Num lugar com poucas redes conhecidas
 * ou internet ruim, passa de trinta segundos com facilidade. Reiniciar o
 * pedido nesse intervalo é garantir que ele nunca termine: cada religada
 * começa a varredura do zero.
 *
 * Com o dobro a cada vez (30 s, 1 min, 2 min, 4 min, 8 min), o religar
 * continua resolvendo o caso para o qual foi criado — o ajuste de precisão
 * ligado com o app aberto — e para de atrapalhar a aquisição normal.
 */
private const val TETO_DA_ESPERA_PARA_RELIGAR_MS = 8 * 60_000L

internal object Localizacao {

    private var manager: LocationManager? = null
    private var ultima: Location? = null
    private var erro: String? = null
    private var ligado = false

    /** Quantos satélites o chip VÊ, e quantos entraram no cálculo. É o número
     *  que separa "sem antena" de "sem céu": ver o comentário do arquivo. */
    @Volatile private var satelitesVisiveis = -1

    @Volatile private var satelitesUsados = -1

    /** O cliente do provedor fundido, quando a ROM tem Play Services. */
    private var fundido: FusedLocationProviderClient? = null

    /** Por que o fundido não subiu. `null` = subiu. */
    @Volatile private var erroFundido: String? = null

    /**
     * O provedor fundido do Google — satélite, Wi-Fi e rede móvel somados.
     *
     * É ele que faz um tablet sem antena de GPS saber onde está: quando o chip
     * não vê satélite, a posição sai dos pontos de Wi-Fi em volta, que o Google
     * mapeou. Nesta central o chip nunca reportou um satélite sequer, então
     * este caminho não é melhoria — é a única chance de haver posição.
     *
     * Chega por aqui e cai no MESMO `ouvinte` do `LocationManager`: o
     * `melhorQue` já sabe escolher entre fontes misturadas, e ter duas regras
     * de escolha seria pior que ter uma.
     */
    private val ouvinteFundido = object : LocationCallback() {
        override fun onLocationResult(resultado: LocationResult) {
            resultado.lastLocation?.let { ouvinte.onLocationChanged(it) }
        }
    }

    private val ouvinte = object : LocationListener {
        override fun onLocationChanged(location: Location) {
            // `synchronized(Localizacao)`, e NÃO `@Synchronized`.
            //
            // `@Synchronized` aqui trancaria este objeto anônimo, enquanto quem
            // lê `ultima` tranca o `Localizacao`. Dois monitores diferentes não
            // estabelecem ordem de memória entre si: a posição escrita aqui
            // podia não ficar visível para quem lê, e a JVM estaria no direito
            // dela. Com o monitor do `Localizacao` os dois lados falam a mesma
            // língua.
            synchronized(Localizacao) {
                // Guarda a melhor entre satélite e rede: as fontes chegam
                // misturadas, e a mais recente nem sempre é a mais precisa.
                val atual = ultima
                ultima = if (atual == null || melhorQue(location, atual)) location else atual
                // Veio posição: se um dia ela sumir de novo, começar a tentar
                // do intervalo curto outra vez.
                esperaParaReligarMs = PACIENCIA_ANTES_DE_RELIGAR_MS
            }
        }

        // Obrigatórios em API < 30; sem eles o Android 10 derruba o registro.
        override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}

        override fun onProviderEnabled(provider: String) {}

        override fun onProviderDisabled(provider: String) {}
    }

    private val satelites = object : GnssStatus.Callback() {
        override fun onSatelliteStatusChanged(status: GnssStatus) {
            satelitesVisiveis = status.satelliteCount
            var usados = 0
            for (i in 0 until status.satelliteCount) {
                if (status.usedInFix(i)) usados++
            }
            satelitesUsados = usados
        }
    }

    /**
     * Uma posição é melhor que a outra se for bem mais nova, ou mais precisa.
     *
     * Dois minutos é o corte clássico do Android: passou disso, o carro andou o
     * suficiente para a posição velha não valer mais, por mais precisa que
     * fosse.
     */
    private fun melhorQue(nova: Location, velha: Location): Boolean {
        val deltaMs = nova.time - velha.time
        if (deltaMs > VELHA_DEMAIS_MS) return true
        if (deltaMs < -VELHA_DEMAIS_MS) return false
        return nova.accuracy <= velha.accuracy
    }

    private fun temPermissao(context: Context): Boolean =
        ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED ||
            ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_COARSE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED

    /** Liga os dois provedores. Idempotente — chamar de novo não duplica. */
    @Synchronized
    fun ligar(context: Context) {
        if (ligado) return

        if (!temPermissao(context)) {
            erro = "sem permissão de localização"
            return
        }

        val lm = context.getSystemService(Context.LOCATION_SERVICE) as? LocationManager
        if (lm == null) {
            erro = "o aparelho não tem LocationManager"
            return
        }
        manager = lm

        // Satélite E rede, os dois ao mesmo tempo. A rede responde em segundos e
        // segura o mapa enquanto o satélite não fixa; o satélite depois ganha no
        // `melhorQue` por precisão.
        var algum = false
        for (provedor in listOf(LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER)) {
            if (!lm.allProviders.contains(provedor)) continue
            try {
                lm.requestLocationUpdates(
                    provedor,
                    INTERVALO_MS,
                    DISTANCIA_MIN_M,
                    ouvinte,
                    Looper.getMainLooper(),
                )
                algum = true
                // Arranque a frio: o Android costuma ter uma posição guardada da
                // última vez que alguém pediu, e ela aparece no mapa na hora.
                lm.getLastKnownLocation(provedor)?.let { ouvinte.onLocationChanged(it) }
            } catch (e: SecurityException) {
                erro = "permissão recusada pelo sistema: ${e.message}"
            } catch (e: Exception) {
                erro = e.message ?: e.javaClass.simpleName
            }
        }

        // A contagem de satélites é o que separa antena de céu. Precisa de
        // Handler próprio: o callback chega na thread do Looper informado.
        try {
            lm.registerGnssStatusCallback(satelites, Handler(Looper.getMainLooper()))
        } catch (e: Exception) {
            // Não é fatal — só perdemos o diagnóstico mais informativo.
            satelitesVisiveis = -2
        }

        val fundidoSubiu = ligarFundido(context)

        // `algum` conta só o `LocationManager`. O fundido sozinho já basta para
        // haver posição — e nesta central ele é a única fonte com chance real.
        if (!algum && !fundidoSubiu && erro == null) {
            erro = "nenhum provedor de localização disponível"
        }
        ligado = algum || fundidoSubiu
        if (ligado) ligadoDesdeMs = System.currentTimeMillis()
    }

    /** Já pedimos? Uma vez por processo — insistir vira diálogo em loop. */
    @Volatile private var pediuPrecisao = false

    /** Quando os provedores foram registrados. Base do religamento. */
    @Volatile private var ligadoDesdeMs = 0L

    /** Quanto esperar antes da PRÓXIMA religada. Dobra a cada uma. */
    @Volatile private var esperaParaReligarMs = PACIENCIA_ANTES_DE_RELIGAR_MS

    /**
     * Pede ao sistema o diálogo de "melhorar a precisão de localização".
     *
     * É o MESMO diálogo que o Google Maps mostra ao abrir, e é o único caminho
     * de um toque: mandar o dono cavar em Configurações > Localização >
     * Precisão do Google é, na prática, não consertar.
     *
     * Quem liga o ajuste é ELE, tocando no diálogo do próprio Android — daqui
     * não se mexe em configuração do aparelho, só se faz o pedido.
     *
     * Por que isto importa nesta central: o diário de 21/09 mostra o provedor
     * `network` DESLIGADO. Sem ele o fundido não tem Wi-Fi para trabalhar e
     * cai no satélite, que aqui nunca viu nada — ou seja, sem este diálogo o
     * provedor fundido não resolve coisa alguma.
     *
     * Só é pedido quando há o que consertar: se o `network` já estiver ligado,
     * o `checkLocationSettings` passa e nenhum diálogo aparece.
     */
    @Synchronized
    fun pedirPrecisao(activity: Activity) {
        if (pediuPrecisao) return
        pediuPrecisao = true
        try {
            val pedido =
                LocationRequest.Builder(Priority.PRIORITY_HIGH_ACCURACY, INTERVALO_MS).build()
            val requisito = LocationSettingsRequest.Builder().addLocationRequest(pedido).build()
            LocationServices.getSettingsClient(activity)
                .checkLocationSettings(requisito)
                .addOnFailureListener { e ->
                    // `ResolvableApiException` = "dá para consertar com um
                    // toque". Qualquer outra coisa não tem diálogo que resolva.
                    if (e is ResolvableApiException) {
                        try {
                            e.startResolutionForResult(activity, CODIGO_PRECISAO)
                        } catch (t: Throwable) {
                            erroFundido = "não deu para abrir o diálogo: ${t.message}"
                        }
                    }
                }
        } catch (e: Throwable) {
            // Sem Play Services não há diálogo — e nem fundido. O
            // `LocationManager` segue sozinho.
            erroFundido = erroFundido ?: (e.message ?: e.javaClass.simpleName)
        }
    }

    /**
     * Liga o provedor fundido. `true` se subiu.
     *
     * `catch (Throwable)` e não `catch (Exception)` de propósito: numa ROM sem
     * Play Services a classe não existe em tempo de execução, e o que vem é
     * `NoClassDefFoundError` — que é `Error`, não `Exception`. Com o `catch`
     * estreito o app morreria inteiro no boot em vez de cair no plano B.
     *
     * Falhar aqui não é fatal: o `LocationManager` continua registrado. Numa
     * central com antena de GPS funcionando, aquele caminho sozinho basta.
     */
    private fun ligarFundido(context: Context): Boolean {
        return try {
            val cliente = LocationServices.getFusedLocationProviderClient(context)
            val pedido =
                LocationRequest.Builder(Priority.PRIORITY_HIGH_ACCURACY, INTERVALO_MS)
                    .setMinUpdateDistanceMeters(DISTANCIA_MIN_M)
                    .build()
            cliente.requestLocationUpdates(pedido, ouvinteFundido, Looper.getMainLooper())
            // Arranque a frio, como no `LocationManager`: o Google costuma ter
            // uma posição guardada, e ela pinta o mapa antes da primeira fixação.
            cliente.lastLocation.addOnSuccessListener { l ->
                l?.let { ouvinte.onLocationChanged(it) }
            }
            fundido = cliente
            erroFundido = null
            true
        } catch (e: SecurityException) {
            erroFundido = "permissão recusada pelo sistema: ${e.message}"
            false
        } catch (e: Throwable) {
            erroFundido = e.message ?: e.javaClass.simpleName
            false
        }
    }

    /**
     * Solta tudo o que estava registrado.
     *
     * Sem isto o religamento empilharia pedidos: o Android guarda um registro
     * por (ouvinte, provedor), e registrar de novo sem remover o anterior
     * deixaria dois pedidos vivos para sempre — cada religada dobrando a conta
     * de bateria por um ganho nenhum.
     */
    @Synchronized
    private fun soltar() {
        try {
            manager?.removeUpdates(ouvinte)
        } catch (e: Throwable) {
            Log.w(TAG, "não consegui soltar o LocationManager: ${e.message}")
        }
        try {
            manager?.unregisterGnssStatusCallback(satelites)
        } catch (e: Throwable) {
            Log.w(TAG, "não consegui soltar o contador de satélites: ${e.message}")
        }
        try {
            fundido?.removeLocationUpdates(ouvinteFundido)
        } catch (e: Throwable) {
            Log.w(TAG, "não consegui soltar o provedor fundido: ${e.message}")
        }
        fundido = null
        ligado = false
    }

    /**
     * Registra tudo de novo quando já faz tempo demais sem posição nenhuma.
     *
     * O caso que isto conserta: o dono abre o app, aceita o diálogo de
     * precisão, e o mapa continua vazio — porque o registro foi feito ANTES de
     * o provedor de rede existir, e ligar o ajuste depois não reentrega nada a
     * quem já estava registrado contra um provedor morto. Antes, a única saída
     * era fechar e abrir o app; o dono descobriu isso sozinho, no carro.
     *
     * Só religa se NUNCA houve posição (`ultima == null`). Com uma posição na
     * mão, mesmo velha, o caminho está funcionando e mexer nele só arriscaria
     * perder o que já se tem.
     */
    @Synchronized
    private fun talvezReligar(context: Context) {
        if (!ligado || ultima != null) return
        val espera = esperaParaReligarMs
        if (System.currentTimeMillis() - ligadoDesdeMs < espera) return

        Log.w(TAG, "sem posição há ${espera}ms; registrando de novo")
        soltar()
        ligar(context)
        // Dobra ANTES da próxima: cada religada custa uma varredura de Wi-Fi
        // começada do zero, e insistir no mesmo ritmo impede a aquisição em
        // vez de ajudá-la.
        esperaParaReligarMs = (espera * 2).coerceAtMost(TETO_DA_ESPERA_PARA_RELIGAR_MS)
    }

    /** O que há de mais recente, para o Rust buscar de tempos em tempos. */
    @Synchronized
    fun ultima(context: Context): JSONObject {
        ligar(context)
        talvezReligar(context)

        val fora = JSONObject()
        val l = ultima
        if (l == null) {
            fora.put("tem", false)
            fora.put("motivo", erro ?: "ainda sem posição")
            // Vai junto de toda recusa: é o que responde POR QUE não veio, e
            // sem isto a resposta do carro é sempre a mesma frase inútil.
            fora.put("diagnostico", diagnostico(context))
            return fora
        }
        fora.put("tem", true)
        fora.put("lat", l.latitude)
        fora.put("lon", l.longitude)
        // `hasSpeed`/`hasBearing` e não o valor seco: parado, o Android devolve
        // zero em vez de "não sei", e zero é uma resposta — o Rust decide.
        fora.put("velocidadeMs", if (l.hasSpeed()) l.speed else 0f)
        fora.put("rumo", if (l.hasBearing()) l.bearing else -1f)
        fora.put("precisaoM", if (l.hasAccuracy()) l.accuracy else -1f)
        fora.put("provedor", l.provider ?: "?")
        // Idade: o Rust precisa saber se está recebendo a mesma posição velha
        // repetidas vezes, que é diferente de não receber nada.
        fora.put("idadeMs", System.currentTimeMillis() - l.time)
        return fora
    }

    /**
     * Por que não veio posição — em fatos, não em adjetivos.
     *
     * Cada campo aqui aponta para um dono diferente do problema: `ligado` falso
     * é configuração do Android (dele), `gps` ausente é ROM, satélites zerados
     * com tudo ligado é antena, e satélites visíveis sem nenhum usado é céu.
     */
    private fun diagnostico(context: Context): JSONObject {
        val d = JSONObject()
        d.put("permissao", temPermissao(context))

        val lm = manager
        if (lm == null) {
            d.put("manager", false)
            return d
        }

        // O interruptor mestre de localização do Android. Se estiver desligado,
        // nenhum provedor entrega nada e não há nada a consertar no código.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            d.put("localizacaoLigada", lm.isLocationEnabled)
        }

        val provs = JSONArray()
        for (p in lm.allProviders) {
            val item = JSONObject().put("nome", p)
            item.put(
                "ligado",
                try {
                    lm.isProviderEnabled(p)
                } catch (e: Exception) {
                    false
                },
            )
            item.put(
                "ultimaConhecida",
                try {
                    val u = lm.getLastKnownLocation(p)
                    if (u == null) {
                        "nunca"
                    } else {
                        "há ${(System.currentTimeMillis() - u.time) / 1000}s"
                    }
                } catch (e: SecurityException) {
                    "sem permissão"
                } catch (e: Exception) {
                    "erro: ${e.javaClass.simpleName}"
                },
            )
            provs.put(item)
        }
        d.put("provedores", provs)

        // -1 = o callback nunca falou (o chip não reportou nada);
        // -2 = nem deu para registrar o callback.
        d.put("satelitesVisiveis", satelitesVisiveis)
        d.put("satelitesUsados", satelitesUsados)
        if (erro != null) d.put("erroAoLigar", erro)
        // O fundido é a fonte com chance real nesta central; se ELE não subiu,
        // é isso que explica a falta de posição, e não os satélites.
        d.put("fundido", if (erroFundido == null) "ligado" else "não: $erroFundido")
        return d
    }
}
