// A posição vinda do Android, e não da WebView.
//
// O `navigator.geolocation` nunca entregou nada nesta central — nem por
// satélite nem por rede, com timeout idêntico nos dois. Esse empate é a
// assinatura de um pedido que NÃO CHEGA ao sistema: a WebView do Android só
// libera geolocalização para a página se o app hospedeiro responder o
// `onGeolocationPermissionsShowPrompt`, e o Tauri não responde. O pedido fica
// pendurado para sempre, que era o comportamento antes de existir o timeout.
//
// Aqui o caminho é direto: `LocationManager` → Kotlin → Rust → módulo `nav`,
// pelo mesmo `push_location` que o JS usava. A WebView sai do meio.
//
// ⚠️ Mora no plugin de Bluetooth porque é aqui que o Eclipse já fala com o
// Android — e porque este plugin já pede ACCESS_FINE_LOCATION para varrer
// adaptador no Android ≤ 11. Um plugin próprio seria mais arrumado; não vale a
// cerimônia agora.

package com.eclipseos.obdbt

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Bundle
import android.os.Looper
import androidx.core.content.ContextCompat
import org.json.JSONObject

/** De quanto em quanto tempo o Android pode mandar posição nova. */
private const val INTERVALO_MS = 1_000L

/** Distância mínima para uma posição nova valer. Zero: quem filtra parada é o
 *  `FiltroDeParada` do Rust, e dois filtros discordando é pior que um. */
private const val DISTANCIA_MIN_M = 0f

internal object Localizacao {

    private var manager: LocationManager? = null
    private var ultima: Location? = null
    private var erro: String? = null
    private var ligado = false

    private val ouvinte = object : LocationListener {
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

    /**
     * Uma posição é melhor que a outra se for bem mais nova, ou mais precisa.
     *
     * Dois minutos é o corte clássico do Android: passou disso, o carro andou o
     * suficiente para a posição velha não valer mais, por mais precisa que
     * fosse.
     */
    private fun melhorQue(nova: Location, velha: Location): Boolean {
        val deltaMs = nova.time - velha.time
        if (deltaMs > 120_000L) return true
        if (deltaMs < -120_000L) return false
        return nova.accuracy <= velha.accuracy
    }

    /** Liga os dois provedores. Idempotente — chamar de novo não duplica. */
    @Synchronized
    fun ligar(context: Context) {
        if (ligado) return

        if (ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) !=
            PackageManager.PERMISSION_GRANTED &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_COARSE_LOCATION) !=
            PackageManager.PERMISSION_GRANTED
        ) {
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
        // `melhorQue` por precisão. Era essa queda que o JS tentava fazer na mão.
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
                // última vez que alguém pediu, e ela aparece no mapa na hora, em
                // vez de deixar a tela vazia até o primeiro fix.
                lm.getLastKnownLocation(provedor)?.let { ouvinte.onLocationChanged(it) }
            } catch (e: SecurityException) {
                erro = "permissão recusada pelo sistema: ${e.message}"
            } catch (e: Exception) {
                erro = e.message ?: e.javaClass.simpleName
            }
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
}
