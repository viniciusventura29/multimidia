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
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject

/** De quanto em quanto tempo o Android pode mandar posição nova. */
private const val INTERVALO_MS = 1_000L

/** Distância mínima para uma posição nova valer. Zero: quem filtra parada é o
 *  `FiltroDeParada` do Rust, e dois filtros discordando é pior que um. */
private const val DISTANCIA_MIN_M = 0f

/** O corte clássico do Android para "a posição velha não vale mais". */
private const val VELHA_DEMAIS_MS = 120_000L

internal object Localizacao {

    private var manager: LocationManager? = null
    private var ultima: Location? = null
    private var erro: String? = null
    private var ligado = false

    /** Quantos satélites o chip VÊ, e quantos entraram no cálculo. É o número
     *  que separa "sem antena" de "sem céu": ver o comentário do arquivo. */
    @Volatile private var satelitesVisiveis = -1

    @Volatile private var satelitesUsados = -1

    private val ouvinte = object : LocationListener {
        @Synchronized
        override fun onLocationChanged(location: Location) {
            // Guarda a melhor entre satélite e rede: os dois provedores chegam
            // misturados, e o mais recente nem sempre é o mais preciso.
            val atual = ultima
            ultima = if (atual == null || melhorQue(location, atual)) location else atual
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

        if (!algum && erro == null) {
            erro = "nenhum provedor de localização disponível"
        }
        ligado = algum
    }

    /** O que há de mais recente, para o Rust buscar de tempos em tempos. */
    @Synchronized
    fun ultima(context: Context): JSONObject {
        ligar(context)

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
        return d
    }
}
