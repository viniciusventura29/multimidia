// Uma sonda, e não uma implementação.
//
// A pergunta que ela responde é uma só: o Spotify aceita o Eclipse como cliente
// de `MediaBrowser`, e o que ele entrega quando aceita? Disso depende a decisão
// de trocar o Web Playback SDK (áudio decodificado dentro da WebView, que é o
// que hoje fica mudo, reinicia sozinho e erra a duração) por MediaController.
//
// Alguns apps liberam a árvore só para pacotes autorizados — o Android Auto, o
// Assistente. Se o Spotify fizer isso, metade do plano morre aqui, e é melhor
// descobrir com trinta linhas do que depois de reescrever o módulo de música.
//
// ⚠️ Mora no plugin de Bluetooth por economia, não por pertencer aqui. É código
// descartável: ou vira um plugin próprio quando a decisão for tomada, ou é
// apagado quando a resposta chegar.

package com.eclipseos.obdbt

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.media.MediaMetadata
import android.media.browse.MediaBrowser
import android.media.session.MediaController
import android.net.Uri
import android.util.Log
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject

/** Quanto esperar a árvore. Conexão local — se demorar isso, não vem. */
private const val ESPERA_MS = 6_000L

/** Quantos filhos da raiz descrever. O suficiente para saber a forma. */
private const val AMOSTRA = 12

internal object SondaMedia {

    /**
     * Procura todo serviço de MediaBrowser instalado e sonda o do Spotify.
     *
     * Enumerar em vez de cravar o nome da classe: o Spotify já mudou esse nome
     * mais de uma vez, e a lista ainda revela que outros tocadores existem no
     * aparelho — que é exatamente o ganho de "serve para qualquer tocador".
     */
    fun sondar(context: Context): JSONObject {
        val resultado = JSONObject()

        val servicos = context.packageManager
            .queryIntentServices(Intent("android.media.browse.MediaBrowserService"), 0)
        val encontrados = JSONArray()
        for (s in servicos) {
            encontrados.put("${s.serviceInfo.packageName}/${s.serviceInfo.name}")
        }
        resultado.put("tocadores", encontrados)

        val spotify = servicos.firstOrNull { it.serviceInfo.packageName.startsWith("com.spotify") }
        if (spotify == null) {
            resultado.put("spotify", "não instalado")
            return resultado
        }

        val componente =
            ComponentName(spotify.serviceInfo.packageName, spotify.serviceInfo.name)
        resultado.put("spotify", componente.flattenToShortString())

        return try {
            resultado.put("arvore", conectarEDescrever(context, componente))
            resultado
        } catch (e: Exception) {
            resultado.put("erro", e.message ?: e.javaClass.simpleName)
            resultado
        }
    }

    private fun conectarEDescrever(context: Context, componente: ComponentName): JSONObject {
        val saida = JSONObject()
        val pronto = CountDownLatch(1)
        var browser: MediaBrowser? = null

        val callback = object : MediaBrowser.ConnectionCallback() {
            override fun onConnected() {
                saida.put("conectou", true)
                pronto.countDown()
            }

            override fun onConnectionFailed() {
                // O cenário que mata a etapa de navegação: o Spotify recusa
                // quem não é Android Auto.
                saida.put("conectou", false)
                saida.put("motivo", "o app recusou a conexão")
                pronto.countDown()
            }

            override fun onConnectionSuspended() {
                saida.put("conectou", false)
                saida.put("motivo", "conexão suspensa")
                pronto.countDown()
            }
        }

        browser = MediaBrowser(context, componente, callback, null)
        browser.connect()

        if (!pronto.await(ESPERA_MS, TimeUnit.MILLISECONDS)) {
            browser.disconnect()
            saida.put("conectou", false)
            saida.put("motivo", "nem conectou nem recusou dentro de ${ESPERA_MS}ms")
            return saida
        }
        if (!saida.optBoolean("conectou", false)) {
            browser.disconnect()
            return saida
        }

        try {
            saida.put("raiz", browser.root)
            saida.put("filhos", filhosDaRaiz(browser))
            // A capa do que está tocando vem por aqui, e não pelo `MediaBrowser`:
            // o token da sessão é entregue junto da conexão, então dá para ler o
            // metadado sem precisar de permissão de acesso a notificações.
            saida.put("tocandoAgora", metadadoAtual(context, browser))
        } finally {
            browser.disconnect()
        }
        return saida
    }

    private fun filhosDaRaiz(browser: MediaBrowser): JSONArray {
        val itens = JSONArray()
        val pronto = CountDownLatch(1)

        browser.subscribe(
            browser.root,
            object : MediaBrowser.SubscriptionCallback() {
                override fun onChildrenLoaded(id: String, filhos: MutableList<MediaBrowser.MediaItem>) {
                    for (item in filhos.take(AMOSTRA)) {
                        val d = item.description
                        itens.put(
                            JSONObject()
                                .put("titulo", d.title?.toString() ?: "")
                                .put("sub", d.subtitle?.toString() ?: "")
                                .put("navegavel", item.isBrowsable)
                                .put("tocavel", item.isPlayable)
                                .put("capa", descreverCapa(d.iconUri, d.iconBitmap)),
                        )
                    }
                    pronto.countDown()
                }

                override fun onError(id: String) {
                    pronto.countDown()
                }
            },
        )

        pronto.await(ESPERA_MS, TimeUnit.MILLISECONDS)
        return itens
    }

    private fun metadadoAtual(context: Context, browser: MediaBrowser): JSONObject {
        val fora = JSONObject()
        return try {
            val controle = MediaController(context, browser.sessionToken)
            val m = controle.metadata ?: return fora.put("estado", "nada tocando")
            fora.put("faixa", m.getString(MediaMetadata.METADATA_KEY_TITLE) ?: "")
            fora.put("artista", m.getString(MediaMetadata.METADATA_KEY_ARTIST) ?: "")
            fora.put("duracaoMs", m.getLong(MediaMetadata.METADATA_KEY_DURATION))
            // A pergunta do dono, em números: a capa grande continua boa?
            fora.put(
                "capa",
                descreverCapa(
                    m.getString(MediaMetadata.METADATA_KEY_ALBUM_ART_URI)?.let(Uri::parse)
                        ?: m.getString(MediaMetadata.METADATA_KEY_ART_URI)?.let(Uri::parse),
                    m.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)
                        ?: m.getBitmap(MediaMetadata.METADATA_KEY_ART),
                ),
            )
            fora
        } catch (e: Exception) {
            fora.put("erro", e.message ?: e.javaClass.simpleName)
        }
    }

    /**
     * Descreve a capa sem trazer a imagem.
     *
     * O que decide o trabalho do lado da tela é o ESQUEMA: `https` a WebView
     * baixa sozinha num `<img>`, como hoje; `content` ela não abre, e aí o
     * bitmap tem que atravessar por um protocolo próprio do Tauri. E o tamanho
     * decide se a capa grande do player continua como está.
     */
    private fun descreverCapa(uri: Uri?, bitmap: Bitmap?): JSONObject {
        val d = JSONObject()
        if (uri != null) {
            d.put("esquema", uri.scheme ?: "?")
            d.put("uri", uri.toString().take(120))
        }
        if (bitmap != null) {
            d.put("bitmap", "${bitmap.width}x${bitmap.height}")
        }
        if (uri == null && bitmap == null) {
            d.put("tem", false)
        }
        return d
    }
}
