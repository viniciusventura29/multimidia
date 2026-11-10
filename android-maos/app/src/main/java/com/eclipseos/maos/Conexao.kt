package com.eclipseos.maos

import android.content.Context
import android.util.Log
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * O WebSocket de SAÍDA até o relay. A central não tem endereço público, então
 * é ela que liga; o relay atende o MCP em nome dela e repassa cada chamada por
 * aqui.
 *
 * Vive enquanto o serviço de acessibilidade estiver ligado: sem ele não há
 * mãos, e não faz sentido aparecer como "conectado".
 */
object Conexao {
    private const val TAG = "maos"

    private val cliente = OkHttpClient.Builder()
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .build()
    private val trabalho = Executors.newCachedThreadPool()
    private val agenda = Executors.newSingleThreadScheduledExecutor()

    @Volatile private var ws: WebSocket? = null
    @Volatile private var ligado = false
    @Volatile private var ultimoPong = 0L
    private var tentativas = 0
    lateinit var app: Context
        private set

    @Volatile var conectado = false
        private set
    @Volatile var ultimoErro: String? = null
        private set

    init {
        // O Eclipse pode abrir depois do Mãos (ou ser reinstalado). Quando as
        // ferramentas dele mudam, anuncia de novo ao relay.
        agenda.scheduleWithFixedDelay({
            if (conectado && Eclipse.atualizar()) ws?.send(ola().toString())
        }, 5, 30, TimeUnit.SECONDS)

        // Ping de aplicação, não de protocolo: o relay responde "pong" sem
        // acordar o Durable Object. Sem pong por 50 s, a conexão morreu calada
        // (troca de rede, hotspot que caiu) e é hora de abrir outra.
        agenda.scheduleWithFixedDelay({
            val atual = ws ?: return@scheduleWithFixedDelay
            if (!conectado) return@scheduleWithFixedDelay
            if (System.currentTimeMillis() - ultimoPong > 50_000) {
                Log.w(TAG, "sem pong há 50 s; reabrindo")
                atual.cancel()
            } else {
                atual.send("ping")
            }
        }, 20, 20, TimeUnit.SECONDS)
    }

    fun ligar(ctx: Context) {
        app = ctx.applicationContext
        synchronized(this) {
            if (ligado) return
            ligado = true
        }
        abrir()
    }

    fun desligar() {
        ligado = false
        ws?.close(1000, "serviço desligado")
        ws = null
        conectado = false
    }

    fun enviar(texto: String) {
        ws?.send(texto)
    }

    fun enviar(bytes: ByteString) {
        ws?.send(bytes)
    }

    private fun abrir() {
        if (!ligado) return
        if (BuildConfig.RELAY.isEmpty()) {
            ultimoErro = "Este APK foi compilado sem o endereço do relay (falta android-maos/maos.properties)."
            return
        }
        val req = Request.Builder()
            .url(BuildConfig.RELAY.trimEnd('/') + "/carro")
            .header("x-carro-chave", BuildConfig.CHAVE)
            .header("x-carro-app", "maos")
            .build()
        ws = cliente.newWebSocket(req, Ouvinte())
    }

    private fun reabrir() {
        if (!ligado) return
        val espera = minOf(30L, 1L shl minOf(tentativas, 5))
        tentativas++
        agenda.schedule({ abrir() }, espera, TimeUnit.SECONDS)
    }

    private class Ouvinte : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            if (webSocket !== ws) return
            conectado = true
            ultimoErro = null
            tentativas = 0
            ultimoPong = System.currentTimeMillis()
            Log.i(TAG, "conectado ao relay")
            trabalho.execute {
                Eclipse.atualizar()
                webSocket.send(ola().toString())
            }
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            if (text == "pong") {
                ultimoPong = System.currentTimeMillis()
                return
            }
            val m = try { JSONObject(text) } catch (e: Exception) { return }
            when (m.optString("tipo")) {
                "chamada" -> trabalho.execute { atender(m) }
                "apk_inicio" -> Instalador.inicio(m.getString("id"), m.optLong("tamanho"))
                "apk_fim" -> trabalho.execute { Instalador.fim(m.getString("id")) }
            }
        }

        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
            Instalador.pedaco(bytes)
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(1000, null)
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            caiu(webSocket, "fechado: $code $reason")
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            val motivo = if (response?.code == 401) "chave recusada pelo relay" else (t.message ?: t.javaClass.simpleName)
            caiu(webSocket, motivo)
        }

        private fun caiu(webSocket: WebSocket, motivo: String) {
            if (webSocket !== ws) return
            conectado = false
            ultimoErro = motivo
            Log.w(TAG, "conexão caiu: $motivo")
            reabrir()
        }
    }

    private fun ola(): JSONObject {
        val lista = JSONArray()
        for (f in Ferramentas.todas) {
            lista.put(JSONObject().apply {
                put("name", f.nome)
                put("description", f.descricao)
                put("inputSchema", f.esquema)
                put("timeout", f.timeoutMs)
            })
        }
        for (f in Eclipse.conhecidas) lista.put(f)
        return JSONObject().apply {
            put("tipo", "ola")
            put("ferramentas", lista)
            put("info", Ferramentas.resumo())
        }
    }

    private fun atender(m: JSONObject) {
        val id = m.getString("id")
        val nome = m.getString("ferramenta")
        val args = m.optJSONObject("args") ?: JSONObject()
        val resultado = try {
            val f = Ferramentas.todas.firstOrNull { it.nome == nome }
            when {
                f != null -> {
                    Ferramentas.travarSeAndando(f)
                    f.executar(args)
                }
                Eclipse.conhece(nome) -> Eclipse.chamar(nome, args)
                else -> throw Falha("ferramenta desconhecida: $nome")
            }
        } catch (e: Falha) {
            Resultado.erro(e.message ?: "falhou")
        } catch (e: Exception) {
            Log.e(TAG, "ferramenta $nome quebrou", e)
            Resultado.erro("${e.javaClass.simpleName}: ${e.message}")
        }
        responder(id, resultado)
    }

    fun responder(id: String, r: Resultado) {
        enviar(JSONObject().apply {
            put("tipo", "resposta")
            put("id", id)
            put("conteudo", r.conteudo)
            put("erro", r.erro)
        }.toString())
    }
}

/** Erro esperado, com mensagem para o Claude ler (não é bug do app). */
class Falha(msg: String) : Exception(msg)

class Resultado private constructor(val conteudo: JSONArray, val erro: Boolean) {
    companion object {
        fun texto(t: String) = Resultado(JSONArray().put(textoJson(t)), false)
        fun erro(t: String) = Resultado(JSONArray().put(textoJson(t)), true)
        fun imagem(base64: String, mime: String, legenda: String) = Resultado(
            JSONArray()
                .put(JSONObject().put("type", "image").put("data", base64).put("mimeType", mime))
                .put(textoJson(legenda)),
            false,
        )

        private fun textoJson(t: String) = JSONObject().put("type", "text").put("text", t)
    }
}
