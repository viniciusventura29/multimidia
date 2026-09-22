// Falar com o app do Spotify que está NESTE aparelho, em vez de com a nuvem.
//
// Hoje toda interação de música é uma ida à Web API do Spotify, pela internet
// do celular preso por tethering. O diário do carro mostra o preço:
//
//     aviso music  o Spotify demorou a responder o toque          {"ms": 1558}
//     aviso music  ler o que está tocando demora mais que o
//                  esperado                                       {"ms": 2112}
//
// Dois segundos para saber o que está tocando, num laço que pergunta a cada
// três. E as chamadas que morrem em timeout de 10 s são a mesma doença pior.
//
// O app do Spotify já está instalado na central e publica uma `MediaSession` —
// o mesmo mecanismo que faz o botão de pausa do volante funcionar. Ela responde
// em microssegundos, não usa rede nenhuma, e entrega a CAPA como bitmap pronto.
//
// ⚠️ ESTE ARQUIVO NÃO SUBSTITUI A WEB API. Ele atende só o que é rápido e local:
// o que está tocando, a capa, e os toques de transporte. Busca e playlists
// continuam na Web API, porque navegar a biblioteca depende de o Spotify
// aceitar o `MediaBrowser` — pergunta que a sonda ainda não respondeu (ela
// estourava antes de perguntar; ver o conserto do `Looper` em `SondaMedia.kt`).
//
// Se a conexão for recusada, tudo aqui devolve `conectado: false` e o Rust
// segue pela Web API. O pior caso é ficar igual a hoje.

package com.eclipseos.obdbt

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.media.MediaMetadata
import android.media.browse.MediaBrowser
import android.media.session.MediaController
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Base64
import android.util.Log
import java.io.ByteArrayOutputStream
import org.json.JSONObject

/** Lado maior da capa que atravessa a ponte. */
private const val CAPA_MAX_PX = 320

/** Qualidade do JPEG da capa. 85 é o joelho da curva: acima disso o arquivo
 *  cresce e o olho não vê diferença numa tela de carro. */
private const val CAPA_QUALIDADE = 85

internal object SessaoMedia {

    @Volatile private var browser: MediaBrowser? = null
    @Volatile private var controle: MediaController? = null

    /** `null` enquanto nunca se tentou; texto = o porquê da última recusa. */
    @Volatile private var motivo: String? = "ainda não tentou"

    /**
     * De qual faixa a capa já foi enviada.
     *
     * A capa é o campo caro — uns 30 KB de base64. Mandá-la a cada leitura, num
     * laço de 1 Hz, seria desperdício puro: ela só muda quando a faixa muda.
     */
    @Volatile private var capaEnviadaDe: String? = null

    /** Conecta se ainda não estiver conectado. Barato e idempotente. */
    private fun garantirConexao(context: Context) {
        if (controle != null) return

        val servicos = context.packageManager
            .queryIntentServices(Intent("android.media.browse.MediaBrowserService"), 0)
        val spotify = servicos.firstOrNull { it.serviceInfo.packageName.startsWith("com.spotify") }
        if (spotify == null) {
            motivo = "o Spotify não está instalado nesta central"
            return
        }
        val componente = ComponentName(spotify.serviceInfo.packageName, spotify.serviceInfo.name)

        // Mesma lição do `SondaMedia`: `MediaBrowser` cria um `Handler` no
        // construtor, e quem chama daqui é uma thread de pool do Tokio, sem
        // `Looper`. Nasce na principal ou não nasce.
        Handler(Looper.getMainLooper()).post {
            try {
                if (controle != null) return@post
                val b = MediaBrowser(
                    context,
                    componente,
                    object : MediaBrowser.ConnectionCallback() {
                        override fun onConnected() {
                            try {
                                val atual = browser ?: return
                                controle = MediaController(context, atual.sessionToken)
                                motivo = null
                            } catch (t: Throwable) {
                                motivo = "conectou mas não deu o controle: ${t.message}"
                            }
                        }

                        override fun onConnectionFailed() {
                            motivo = "o Spotify recusou a conexão"
                            derrubar()
                        }

                        override fun onConnectionSuspended() {
                            motivo = "conexão suspensa"
                            derrubar()
                        }
                    },
                    null,
                )
                browser = b
                b.connect()
            } catch (t: Throwable) {
                motivo = "não deu para criar o MediaBrowser: ${t.message}"
                derrubar()
            }
        }
    }

    /** Solta tudo, para a próxima leitura tentar de novo do zero. */
    private fun derrubar() {
        controle = null
        try {
            browser?.disconnect()
        } catch (t: Throwable) {
            Log.w(TAG, "falha ao soltar o MediaBrowser: ${t.message}")
        }
        browser = null
    }

    /**
     * O que está tocando AGORA, direto do app ao lado.
     *
     * Devolve sempre um objeto: `conectado: false` com um motivo é uma resposta
     * legítima, e é ela que manda o Rust seguir pela Web API.
     */
    fun estado(context: Context): JSONObject {
        val fora = JSONObject()
        return try {
            garantirConexao(context)
            val c = controle
            if (c == null) {
                fora.put("conectado", false)
                fora.put("motivo", motivo ?: "conectando")
                return fora
            }
            fora.put("conectado", true)

            val m = c.metadata
            val estado = c.playbackState
            if (m == null) {
                // Conectado e sem metadado = o Spotify está aberto e parado.
                // Não é erro, e não deve derrubar a conexão.
                fora.put("tem", false)
                fora.put("motivo", "nada tocando")
                return fora
            }

            fora.put("tem", true)
            fora.put("faixa", m.getString(MediaMetadata.METADATA_KEY_TITLE) ?: "")
            fora.put("artista", m.getString(MediaMetadata.METADATA_KEY_ARTIST) ?: "")
            val duracao = m.getLong(MediaMetadata.METADATA_KEY_DURATION)
            if (duracao > 0) fora.put("duracaoMs", duracao)

            if (estado != null) {
                fora.put("tocando", estado.state == PlaybackState.STATE_PLAYING)
                fora.put("posicaoMs", posicaoAgora(estado))
            }

            // A capa só quando a faixa muda — ver `capaEnviadaDe`.
            val id = identidade(m)
            if (id != null && id != capaEnviadaDe) {
                val capa = capaEmBase64(m)
                if (capa != null) {
                    fora.put("capa", capa)
                    capaEnviadaDe = id
                }
            }
            fora
        } catch (t: Throwable) {
            // Uma leitura que falha não pode derrubar o módulo de música: o Rust
            // lê isto como "sem sessão local" e vai pela Web API.
            derrubar()
            motivo = t.message ?: t.javaClass.simpleName
            JSONObject().put("conectado", false).put("motivo", motivo)
        }
    }

    /**
     * A posição real, e não a do último aviso.
     *
     * O `PlaybackState` guarda a posição de quando foi ATUALIZADO, não de
     * agora. Tocando, o relógio andou desde então — sem somar esse tempo a
     * barra de progresso ficaria congelada entre um aviso e outro.
     */
    private fun posicaoAgora(estado: PlaybackState): Long {
        if (estado.state != PlaybackState.STATE_PLAYING) return estado.position
        val desde = SystemClock.elapsedRealtime() - estado.lastPositionUpdateTime
        return estado.position + (desde * estado.playbackSpeed).toLong()
    }

    /** O que identifica a faixa, para saber se a capa precisa ir de novo. */
    private fun identidade(m: MediaMetadata): String? {
        val id = m.getString(MediaMetadata.METADATA_KEY_MEDIA_ID)
        if (!id.isNullOrEmpty()) return id
        val titulo = m.getString(MediaMetadata.METADATA_KEY_TITLE) ?: return null
        return titulo + "|" + (m.getString(MediaMetadata.METADATA_KEY_ARTIST) ?: "")
    }

    /**
     * A capa como `data:` URI.
     *
     * Vai em base64 de propósito: o bitmap vive na memória do app, e a `content://`
     * que o Spotify às vezes oferece a WebView não sabe abrir. Um `data:` URI
     * entra num `<img src>` sem protocolo novo nem permissão extra.
     *
     * Reduzida antes de codificar — a original costuma vir em 640x640, o que
     * daria uns 120 KB de base64 a cada troca de faixa, para uma capa que na
     * tela do carro não passa de uns 300 px.
     */
    private fun capaEmBase64(m: MediaMetadata): String? {
        val bitmap = m.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)
            ?: m.getBitmap(MediaMetadata.METADATA_KEY_ART)
            ?: return null
        return try {
            val maior = maxOf(bitmap.width, bitmap.height)
            val usar = if (maior > CAPA_MAX_PX) {
                val escala = CAPA_MAX_PX.toFloat() / maior
                Bitmap.createScaledBitmap(
                    bitmap,
                    (bitmap.width * escala).toInt().coerceAtLeast(1),
                    (bitmap.height * escala).toInt().coerceAtLeast(1),
                    true,
                )
            } else {
                bitmap
            }
            val saco = ByteArrayOutputStream()
            usar.compress(Bitmap.CompressFormat.JPEG, CAPA_QUALIDADE, saco)
            if (usar !== bitmap) usar.recycle()
            "data:image/jpeg;base64," + Base64.encodeToString(saco.toByteArray(), Base64.NO_WRAP)
        } catch (t: Throwable) {
            Log.w(TAG, "não deu para converter a capa: ${t.message}")
            null
        }
    }

    /**
     * Um toque de transporte. `true` = a sessão local deu conta.
     *
     * `false` faz o Rust repetir o mesmo comando pela Web API, então errar aqui
     * custa lentidão, não o comando perdido.
     */
    fun comando(context: Context, acao: String, valor: Long): Boolean {
        return try {
            garantirConexao(context)
            val c = controle ?: return false
            val t = c.transportControls
            when (acao) {
                "tocar" -> t.play()
                "pausar" -> t.pause()
                "alternar" -> {
                    if (c.playbackState?.state == PlaybackState.STATE_PLAYING) t.pause() else t.play()
                }
                "proxima" -> t.skipToNext()
                "anterior" -> t.skipToPrevious()
                "saltar" -> t.seekTo(valor)
                else -> return false
            }
            true
        } catch (t: Throwable) {
            Log.w(TAG, "comando '$acao' falhou na sessão local: ${t.message}")
            derrubar()
            false
        }
    }
}
