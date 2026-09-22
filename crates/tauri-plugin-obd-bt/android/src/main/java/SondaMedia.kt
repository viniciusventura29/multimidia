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
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
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

    /**
     * Conecta e descreve — TUDO na thread principal.
     *
     * O `MediaBrowser` cria um `Handler` no construtor, e `Handler` exige um
     * `Looper` na thread onde nasce. O Rust chama esta sonda de uma thread de
     * pool do Tokio, que não tem `Looper` nenhum — e o construtor estourava
     * antes de qualquer conexão:
     *
     *     Can't create handler inside thread Thread[pool-1-thread-1,5,main]
     *     that has not called Looper.prepare()
     *
     * Era por isso que a sonda voltava só com a lista de serviços e um `erro`:
     * ela nunca chegou a perguntar nada ao Spotify. A resposta que ela existe
     * para dar continuava desconhecida.
     *
     * A correção não é `Looper.prepare()` na thread do pool — isso prenderia
     * uma thread do Tokio girando um loop de mensagens. É fazer o trabalho onde
     * já existe um `Looper` vivo: o principal. A thread que chamou fica
     * bloqueada na trava, que é exatamente o que ela já fazia.
     *
     * Toda a descrição acontece DENTRO do `onConnected`, que o Android entrega
     * na thread principal. Assim não há um segundo salto de thread nem leitura
     * de `browser` fora dela.
     */
    private fun conectarEDescrever(context: Context, componente: ComponentName): JSONObject {
        val principal = Handler(Looper.getMainLooper())
        val pronto = CountDownLatch(1)
        val ref = AtomicReference<MediaBrowser?>()
        // O JSON que os callbacks preenchem pertence à thread PRINCIPAL e só a
        // ela. O chamador nunca o toca; ele chega aqui por `entregue`, publicado
        // no mesmo instante em que a trava abre.
        //
        // É isto que tira a corrida por construção: no caminho do timeout, a
        // principal pode continuar escrevendo no objeto dela à vontade, porque
        // quem esperou devolve um objeto PRÓPRIO e nunca lê aquele.
        val entregue = AtomicReference<JSONObject?>()
        val encerrado = AtomicBoolean(false)
        val saida = JSONObject()

        fun encerrar() {
            if (!encerrado.compareAndSet(false, true)) return
            try {
                ref.get()?.disconnect()
            } catch (e: Exception) {
                Log.w(TAG, "falha ao desconectar a sonda: ${e.message}")
            }
            entregue.set(saida)
            pronto.countDown()
        }

        val callback = object : MediaBrowser.ConnectionCallback() {
            override fun onConnected() {
                val b = ref.get()
                if (b == null) {
                    saida.put("conectou", false)
                    saida.put("motivo", "conectou sem browser — não devia acontecer")
                    encerrar()
                    return
                }
                saida.put("conectou", true)
                try {
                    val raiz = b.root
                    saida.put("raiz", raiz)
                    // A capa do que está tocando vem daqui, e não do
                    // `MediaBrowser`: o token da sessão é entregue junto da
                    // conexão, então dá para ler o metadado sem precisar de
                    // permissão de acesso a notificações.
                    saida.put("tocandoAgora", metadadoAtual(context, b))
                    // `subscribe` é assíncrono e responde nesta mesma thread; o
                    // encerramento mora nos callbacks dele.
                    b.subscribe(raiz, assinatura(saida) { encerrar() })
                } catch (e: Exception) {
                    saida.put("erroAoDescrever", e.message ?: e.javaClass.simpleName)
                    encerrar()
                }
            }

            override fun onConnectionFailed() {
                // O cenário que mata a etapa de navegação: o Spotify recusa
                // quem não é Android Auto.
                saida.put("conectou", false)
                saida.put("motivo", "o app recusou a conexão")
                encerrar()
            }

            override fun onConnectionSuspended() {
                saida.put("conectou", false)
                saida.put("motivo", "conexão suspensa")
                encerrar()
            }
        }

        principal.post {
            try {
                val b = MediaBrowser(context, componente, callback, null)
                ref.set(b)
                b.connect()
            } catch (e: Exception) {
                // Agora isto é uma falha de verdade do `MediaBrowser`, e não a
                // thread errada.
                saida.put("conectou", false)
                saida.put("motivo", "não deu para criar o MediaBrowser: ${e.message}")
                encerrar()
            }
        }

        if (pronto.await(ESPERA_MS, TimeUnit.MILLISECONDS)) {
            // A trava abriu: `entregue` foi publicado antes do `countDown`, e a
            // principal já largou o objeto.
            return entregue.get() ?: JSONObject().put("motivo", "sonda sem resposta")
        }

        // Estourou o prazo. Solta o browser na thread dona e devolve um objeto
        // NOVO — o da principal pode estar sendo escrito neste exato momento.
        principal.post {
            if (encerrado.compareAndSet(false, true)) {
                try {
                    ref.get()?.disconnect()
                } catch (e: Exception) {
                    Log.w(TAG, "falha ao desconectar a sonda expirada: ${e.message}")
                }
            }
        }
        return JSONObject()
            .put("conectou", false)
            .put("motivo", "nem conectou nem recusou dentro de ${ESPERA_MS}ms")
    }

    /**
     * O ouvinte da árvore. Fora do `onConnected` só para ele não crescer
     * demais; roda na mesma thread principal.
     */
    private fun assinatura(saida: JSONObject, encerrar: () -> Unit) =
        object : MediaBrowser.SubscriptionCallback() {
            override fun onChildrenLoaded(id: String, filhos: MutableList<MediaBrowser.MediaItem>) {
                saida.put("filhos", descrever(filhos))
                encerrar()
            }

            override fun onError(id: String) {
                // Conectar e não deixar navegar é uma resposta — e é a resposta
                // ruim, a que decide se metade do plano morre.
                saida.put("filhos", JSONArray())
                saida.put("motivoDaArvore", "o app conectou mas recusou listar a raiz")
                encerrar()
            }
        }

    /** Só a descrição dos itens — a espera agora mora na trava do chamador. */
    private fun descrever(filhos: List<MediaBrowser.MediaItem>): JSONArray {
        val itens = JSONArray()
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
