// Falar com o app do Spotify DESTA central, direto.
//
// # Por que isto existe
//
// Três versões tentaram mandar o som para o app da central pela Web API, e
// todas falharam pelo mesmo motivo estrutural: para a Web API comandar um
// aparelho, ele precisa estar anunciado como dispositivo do Spotify Connect —
// e o app do Spotify só se anuncia depois de ser ABERTO na mão.
//
// O dono recusou esse fluxo, com razão: "eu n quero ter que ligar o carro ->
// abrir spotify -> abrir o eclipse OS".
//
// Duas tentativas de contornar isso morreram:
//
// 1. Casar o nome do aparelho com o do dispositivo. O Android chama a central
//    de "K706" em todos os campos que conhece (`DEVICE_NAME`, `MODEL`,
//    `DEVICE`, `PRODUCT` — todos iguais); o Spotify se anuncia como
//    "HT-9960CA". Não há relação entre os dois.
//
// 2. Acordar o app ligando no `MediaBrowserService`. Eu acreditei que
//    funcionava porque um dispositivo apareceu ~51 s depois de um bind no
//    diário de 24/09 — mas o de 25/09 mostra o bind acontecendo e NENHUM
//    dispositivo novo surgindo. Era correlação: o dono tinha aberto o Spotify
//    na mão naquele minuto.
//
// O App Remote não depende de nada disso. Ele inicia o processo do Spotify
// sozinho e manda tocar direto no aparelho onde está rodando. Não há lista de
// dispositivos, não há nome para casar, e não há risco de o som sair no
// computador de casa — coisa que já aconteceu.
//
// # O que ele NÃO faz
//
// Navegar a biblioteca. Busca, playlists e histórico continuam na Web API, que
// faz isso bem. Este arquivo cuida de TOCAR e CONTROLAR.
//
// # Exige cadastro
//
// O pacote `com.eclipseos.app` e o SHA1 do certificado precisam estar no painel
// de desenvolvedor do Spotify. Já estão — ver `libs/PROCEDENCIA.md`.

package com.eclipseos.obdbt

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.spotify.android.appremote.api.ConnectionParams
import com.spotify.android.appremote.api.Connector
import com.spotify.android.appremote.api.SpotifyAppRemote
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONObject

/**
 * Quanto esperar a conexão com o app do Spotify.
 *
 * Generoso porque a primeira conexão pode ter de INICIAR o app: o Android sobe
 * o processo, o Spotify restaura a sessão e só então atende. Depois disso as
 * conexões são instantâneas, e este prazo nunca é alcançado.
 */
private const val ESPERA_CONEXAO_MS = 12_000L

/** Quanto esperar um comando (tocar, pausar) responder. Chamada local. */
private const val ESPERA_COMANDO_MS = 6_000L

internal object AppRemoteSpotify {

    @Volatile private var remoto: SpotifyAppRemote? = null

    /** Por que a última tentativa não deu. `null` = deu. */
    @Volatile private var motivo: String? = "ainda não tentou"

    /** Uma conexão de cada vez: duas em voo derrubariam uma à outra. */
    private val conectando = AtomicBoolean(false)

    /**
     * Garante a conexão, esperando por ela.
     *
     * Bloqueia o chamador de propósito: quem chama é uma thread de pool do
     * Rust, e o comando seguinte não faz sentido sem a conexão de pé.
     */
    private fun garantir(context: Context, clientId: String, redirectUri: String): SpotifyAppRemote? {
        remoto?.let { if (it.isConnected) return it }

        if (!SpotifyAppRemote.isSpotifyInstalled(context)) {
            motivo = "o Spotify não está instalado nesta central"
            return null
        }

        // Se outra thread já está conectando, esperar a dela em vez de abrir
        // uma segunda — o SDK não gosta de conexões concorrentes.
        if (!conectando.compareAndSet(false, true)) {
            Thread.sleep(500)
            return remoto?.takeIf { it.isConnected }
        }

        val pronto = CountDownLatch(1)
        val saida = AtomicReference<SpotifyAppRemote?>()

        val params = ConnectionParams.Builder(clientId)
            .setRedirectUri(redirectUri)
            // `false`: num carro em movimento não se abre tela de login. Se o
            // dono não estiver logado no app do Spotify, a conexão falha e o
            // Rust cai na Web API — que é o comportamento de hoje.
            .showAuthView(false)
            .build()

        // O SDK cria Handler internamente: nasce na thread principal ou não
        // nasce. Mesma lição do `MediaBrowser` (ver `SondaMedia.kt`).
        Handler(Looper.getMainLooper()).post {
            try {
                SpotifyAppRemote.connect(
                    context,
                    params,
                    object : Connector.ConnectionListener {
                        override fun onConnected(r: SpotifyAppRemote) {
                            saida.set(r)
                            motivo = null
                            pronto.countDown()
                        }

                        override fun onFailure(erro: Throwable) {
                            motivo = erro.message ?: erro.javaClass.simpleName
                            Log.w(TAG, "App Remote recusou: $motivo")
                            pronto.countDown()
                        }
                    },
                )
            } catch (t: Throwable) {
                motivo = "não deu para conectar: ${t.message}"
                pronto.countDown()
            }
        }

        val chegou = pronto.await(ESPERA_CONEXAO_MS, TimeUnit.MILLISECONDS)
        if (!chegou) motivo = "o Spotify não respondeu em ${ESPERA_CONEXAO_MS}ms"
        remoto = saida.get()
        conectando.set(false)
        return remoto
    }

    /**
     * Manda tocar. `uri` é `spotify:track:…`, `:album:…` ou `:playlist:…`.
     *
     * Com `contexto` e `uri` de faixa, entra no contexto e pula para a faixa —
     * é o que dá fila de verdade, para "próxima" ter para onde ir.
     */
    fun tocar(
        context: Context,
        clientId: String,
        redirectUri: String,
        uri: String?,
        contexto: String?,
        indice: Int,
    ): JSONObject {
        val r = garantir(context, clientId, redirectUri)
            ?: return JSONObject().put("ok", false).put("motivo", motivo ?: "sem conexão")

        return try {
            val player = r.playerApi
            val chamada = when {
                // Dentro de uma playlist/álbum, na faixa escolhida.
                contexto != null && indice >= 0 -> player.skipToIndex(contexto, indice)
                contexto != null -> player.play(contexto)
                uri != null -> player.play(uri)
                else -> return JSONObject().put("ok", false).put("motivo", "nada para tocar")
            }
            esperar(chamada)
        } catch (t: Throwable) {
            JSONObject().put("ok", false).put("motivo", t.message ?: t.javaClass.simpleName)
        }
    }

    /** Um toque de transporte. */
    fun comando(
        context: Context,
        clientId: String,
        redirectUri: String,
        acao: String,
        valor: Long,
    ): JSONObject {
        val r = garantir(context, clientId, redirectUri)
            ?: return JSONObject().put("ok", false).put("motivo", motivo ?: "sem conexão")

        return try {
            val p = r.playerApi
            val chamada = when (acao) {
                "tocar" -> p.resume()
                "pausar" -> p.pause()
                "proxima" -> p.skipNext()
                "anterior" -> p.skipPrevious()
                "saltar" -> p.seekTo(valor)
                else -> return JSONObject().put("ok", false).put("motivo", "ação desconhecida: $acao")
            }
            esperar(chamada)
        } catch (t: Throwable) {
            JSONObject().put("ok", false).put("motivo", t.message ?: t.javaClass.simpleName)
        }
    }

    /**
     * Espera a chamada do SDK responder.
     *
     * `await(prazo)` e não callbacks: o `CallResult` do App Remote já oferece
     * uma espera com prazo, e quem chama aqui é uma thread de pool do Rust,
     * que pode bloquear. A primeira versão disto montava duas callbacks e uma
     * trava na mão para fazer o que esta linha faz.
     *
     * Esperar importa: sem isso o Rust receberia "deu certo" antes de o
     * Spotify ter feito qualquer coisa, e um erro de reprodução viraria
     * silêncio em vez de mensagem.
     */
    private fun esperar(chamada: com.spotify.protocol.client.CallResult<*>): JSONObject {
        val r = chamada.await(ESPERA_COMANDO_MS, TimeUnit.MILLISECONDS)
        return if (r.isSuccessful) {
            JSONObject().put("ok", true)
        } else {
            JSONObject()
                .put("ok", false)
                .put("motivo", r.errorMessage ?: r.error?.message ?: "o Spotify recusou o comando")
        }
    }
}
