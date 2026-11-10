package com.eclipseos.maos

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * O catálogo MCP do próprio Eclipse, lido pela loopback da central
 * (`src-tauri/src/mcp_local.rs`): telemetria do OBD, posição e rota, música,
 * mensagens e o estado de cada módulo.
 *
 * O Eclipse não fala com o relay porque o APK dele é público e a chave não
 * pode ir junto; o Mãos repassa as ferramentas dele como se fossem suas.
 */
object Eclipse {
    private const val URL = "http://127.0.0.1:47823/mcp"
    private val json = "application/json".toMediaType()
    private val cliente = OkHttpClient.Builder()
        .connectTimeout(2, TimeUnit.SECONDS)
        .readTimeout(20, TimeUnit.SECONDS)
        .build()

    /**
     * As ferramentas do Eclipse já vistas. Ficam anunciadas mesmo com o Eclipse
     * fechado: a chamada é que responde que ele não está aberto, o que diz ao
     * Claude o que fazer (abrir o app) em vez de a ferramenta sumir.
     */
    @Volatile var conhecidas: List<JSONObject> = emptyList()
        private set

    /** Pergunta a lista ao Eclipse. Devolve true se mudou desde a última vez. */
    fun atualizar(): Boolean {
        val lista = try {
            rpc("tools/list", JSONObject()).optJSONObject("result")?.optJSONArray("tools") ?: return false
        } catch (e: Exception) {
            return false
        }
        val novas = (0 until lista.length()).map { lista.getJSONObject(it) }
        val mudou = novas.map { it.optString("name") } != conhecidas.map { it.optString("name") }
        conhecidas = novas
        return mudou
    }

    fun conhece(nome: String) = conhecidas.any { it.optString("name") == nome }

    fun chamar(nome: String, args: JSONObject): Resultado {
        val r = try {
            rpc("tools/call", JSONObject().put("name", nome).put("arguments", args))
        } catch (e: Exception) {
            throw Falha("O Eclipse não está aberto na central. Abra com `abrir_app {pacote: \"com.eclipseos.app\"}` e tente de novo.")
        }
        r.optJSONObject("error")?.let { throw Falha("O Eclipse recusou: ${it.optString("message")}") }
        val res = r.getJSONObject("result")
        val conteudo = res.optJSONArray("content") ?: JSONArray()
        val texto = (0 until conteudo.length()).joinToString("\n") { conteudo.getJSONObject(it).optString("text") }
        return if (res.optBoolean("isError")) Resultado.erro(texto) else Resultado.texto(texto)
    }

    /**
     * Velocidade pelo OBD, ou null se não há como saber (Eclipse fechado,
     * adaptador fora). Null libera o toque: sem leitura, o caso comum é o
     * dono desenvolvendo com o carro parado na garagem.
     */
    fun velocidadeKmh(): Long? = try {
        val r = rpc("tools/call", JSONObject().put("name", "carro_telemetria").put("arguments", JSONObject()))
        val texto = r.getJSONObject("result").getJSONArray("content").getJSONObject(0).getString("text")
        val t = JSONObject(texto)
        if (t.isNull("velocidade_kmh")) null else t.getLong("velocidade_kmh")
    } catch (e: Exception) {
        null
    }

    private fun rpc(metodo: String, params: JSONObject): JSONObject {
        val corpo = JSONObject().put("jsonrpc", "2.0").put("id", 1).put("method", metodo).put("params", params)
        cliente.newCall(Request.Builder().url(URL).post(corpo.toString().toRequestBody(json)).build()).execute().use { r ->
            if (!r.isSuccessful) throw IllegalStateException("HTTP ${r.code}")
            return JSONObject(r.body!!.string())
        }
    }
}
