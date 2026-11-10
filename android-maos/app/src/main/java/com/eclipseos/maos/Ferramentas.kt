package com.eclipseos.maos

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Path
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.util.DisplayMetrics
import android.view.WindowManager
import android.view.accessibility.AccessibilityNodeInfo
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class Ferramenta(
    val nome: String,
    val descricao: String,
    val esquema: JSONObject,
    val timeoutMs: Int = 60_000,
    val executar: (JSONObject) -> Resultado,
)

/**
 * O que o Claude pode fazer na central. Cada uma vira uma ferramenta MCP no
 * relay; a descrição é o que o Claude lê para decidir, então diz quando usar.
 */
object Ferramentas {

    val todas: List<Ferramenta> = listOf(
        Ferramenta(
            "print",
            "Tira print da tela inteira da central (qualquer app, inclusive o mapa). Use para VER o que está na tela. " +
                "Na primeira vez depois de ligar a central, o Android pede permissão e o Mãos aceita sozinho.",
            esquema(
                "max_largura" to prop("integer", "Largura máxima da imagem em pixels (padrão 1280)."),
                "qualidade" to prop("integer", "Qualidade do JPEG, 10 a 100 (padrão 75)."),
            ),
            timeoutMs = 30_000,
        ) { a ->
            Captura.garantir(Conexao.app)
            val (img, legenda) = Captura.tirar(a.optInt("max_largura", 1280), a.optInt("qualidade", 75).coerceIn(10, 100))
            Resultado.imagem(img, "image/jpeg", legenda)
        },

        Ferramenta(
            "tela",
            "Lista os elementos de todas as janelas visíveis: texto, id, posição (esquerda,topo,direita,baixo) e se é clicável, " +
                "rolável ou editável. Cada um ganha um número #N que `tocar` e `digitar` aceitam até o próximo `tela`. " +
                "Mais barato e mais exato que `print` para achar onde tocar.",
            esquema(),
        ) { Resultado.texto(Tela.descrever()) },

        Ferramenta(
            "tocar",
            "Toca na tela. Diga ONDE de um jeito: {x,y} em pixels da tela; {no} com o número do último `tela`; " +
                "{texto} com o texto visível (ou a descrição); ou {id}. Com {longo:true} é toque longo.",
            esquema(
                "x" to prop("number", "Pixel horizontal."),
                "y" to prop("number", "Pixel vertical."),
                "no" to prop("integer", "Número #N do último `tela`."),
                "texto" to prop("string", "Texto visível do elemento (aceita parte do texto)."),
                "id" to prop("string", "Id do elemento, completo ou só o final."),
                "longo" to prop("boolean", "Toque longo (~0,8 s)."),
            ),
        ) { a ->
            val (x, y, alvo) = ponto(a)
            val dur = if (a.optBoolean("longo")) 800L else 60L
            gesto(traco(x, y, x, y, dur))
            Resultado.texto("Toquei${if (a.optBoolean("longo")) " (longo)" else ""} $alvo.")
        },

        Ferramenta(
            "arrastar",
            "Arrasta o dedo de (x1,y1) até (x2,y2). Serve para rolar listas, deslizar e mover o mapa. " +
                "Para rolar para baixo, arraste de baixo para cima.",
            esquema(
                "x1" to prop("number", ""), "y1" to prop("number", ""),
                "x2" to prop("number", ""), "y2" to prop("number", ""),
                "ms" to prop("integer", "Duração em ms (padrão 300; mais lento = menos inércia)."),
                obrigatorios = listOf("x1", "y1", "x2", "y2"),
            ),
        ) { a ->
            val ms = a.optLong("ms", 300).coerceIn(50, 10_000)
            gesto(traco(a.getDouble("x1").toFloat(), a.getDouble("y1").toFloat(), a.getDouble("x2").toFloat(), a.getDouble("y2").toFloat(), ms))
            Resultado.texto("Arrastei de (${a.get("x1")},${a.get("y1")}) até (${a.get("x2")},${a.get("y2")}) em $ms ms.")
        },

        Ferramenta(
            "pinca",
            "Movimento de pinça com dois dedos em volta de (x,y): {abrir:true} afasta os dedos (zoom in), {abrir:false} aproxima (zoom out).",
            esquema(
                "x" to prop("number", "Centro."), "y" to prop("number", "Centro."),
                "abrir" to prop("boolean", "true = zoom in; false = zoom out."),
                "distancia" to prop("number", "Quanto cada dedo anda, em pixels (padrão 200)."),
                obrigatorios = listOf("x", "y", "abrir"),
            ),
        ) { a ->
            val x = a.getDouble("x").toFloat()
            val y = a.getDouble("y").toFloat()
            val d = a.optDouble("distancia", 200.0).toFloat()
            val perto = 30f
            val (de, ate) = if (a.getBoolean("abrir")) perto to perto + d else perto + d to perto
            val g = GestureDescription.Builder()
                .addStroke(GestureDescription.StrokeDescription(linha(x - de, y, x - ate, y), 0, 400))
                .addStroke(GestureDescription.StrokeDescription(linha(x + de, y, x + ate, y), 0, 400))
                .build()
            gesto(g)
            Resultado.texto("Pinça ${if (a.getBoolean("abrir")) "abrindo" else "fechando"} em ($x,$y).")
        },

        Ferramenta(
            "digitar",
            "Escreve texto num campo. Sem alvo, usa o campo com foco (toque nele antes). Também aceita {no}, {id} ou {texto} " +
                "para escolher o campo. Substitui o que havia, a não ser com {anexar:true}.",
            esquema(
                "valor" to prop("string", "O texto a escrever."),
                "no" to prop("integer", ""), "id" to prop("string", ""), "texto" to prop("string", "Texto/descrição do campo."),
                "anexar" to prop("boolean", "Acrescenta ao fim em vez de substituir."),
                obrigatorios = listOf("valor"),
            ),
        ) { a ->
            val campo = Tela.achar(a) ?: Tela.campoComFoco()
                ?: throw Falha("Nenhum campo com foco. Toque no campo antes, ou passe {no}/{id}/{texto}.")
            val valor = a.getString("valor")
            val final = if (a.optBoolean("anexar")) (campo.text?.toString() ?: "") + valor else valor
            val ok = campo.performAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, final) },
            )
            if (!ok) throw Falha("O campo ${Tela.rotulo(campo)} recusou o texto.")
            Resultado.texto("Escrevi \"$final\" em ${Tela.rotulo(campo)}. (No Android 10 não há \"Enter\" por acessibilidade: toque no botão de buscar/confirmar.)")
        },

        Ferramenta(
            "tecla",
            "Botões do sistema: voltar, home, recentes, notificacoes (abre a cortina), config_rapidas.",
            esquema(
                "qual" to prop("string", "voltar | home | recentes | notificacoes | config_rapidas"),
                obrigatorios = listOf("qual"),
            ),
        ) { a ->
            val acao = when (a.getString("qual")) {
                "voltar" -> AccessibilityService.GLOBAL_ACTION_BACK
                "home" -> AccessibilityService.GLOBAL_ACTION_HOME
                "recentes" -> AccessibilityService.GLOBAL_ACTION_RECENTS
                "notificacoes" -> AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS
                "config_rapidas" -> AccessibilityService.GLOBAL_ACTION_QUICK_SETTINGS
                else -> throw Falha("Tecla desconhecida: ${a.getString("qual")}")
            }
            if (!Acessibilidade.exigir().performGlobalAction(acao)) throw Falha("O Android recusou a tecla.")
            Resultado.texto("Apertei ${a.getString("qual")}.")
        },

        Ferramenta(
            "abrir_app",
            "Abre um app pelo pacote (veja `apps`), ou abre uma URL/deep link com {url}. O Eclipse é com.eclipseos.app.",
            esquema(
                "pacote" to prop("string", "Ex.: com.spotify.music"),
                "url" to prop("string", "Ex.: https://… ou spotify:…"),
            ),
        ) { a ->
            val ctx = Conexao.app
            val intent = when {
                a.has("url") -> Intent(Intent.ACTION_VIEW, Uri.parse(a.getString("url"))).apply {
                    if (a.has("pacote")) setPackage(a.getString("pacote"))
                }
                a.has("pacote") -> ctx.packageManager.getLaunchIntentForPackage(a.getString("pacote"))
                    ?: throw Falha("${a.getString("pacote")} não está instalado ou não tem tela inicial.")
                else -> throw Falha("Passe {pacote} ou {url}.")
            }
            ctx.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            Resultado.texto("Abri ${a.optString("url").ifEmpty { a.getString("pacote") }}.")
        },

        Ferramenta(
            "apps",
            "Lista os apps instalados que abrem pela tela inicial: nome, pacote e versão.",
            esquema(),
        ) {
            val pm = Conexao.app.packageManager
            val lista = pm.queryIntentActivities(Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER), 0)
                .map { it.activityInfo.packageName to it.loadLabel(pm).toString() }
                .distinctBy { it.first }
                .sortedBy { it.second.lowercase() }
                .joinToString("\n") { (pacote, nome) ->
                    val v = try { pm.getPackageInfo(pacote, 0).versionName } catch (e: PackageManager.NameNotFoundException) { "?" }
                    "$nome — $pacote ($v)"
                }
            Resultado.texto(lista)
        },

        Ferramenta(
            "esperar_tela",
            "Espera até um texto aparecer em qualquer janela (ou sumir, com {sumir:true}). Use depois de abrir um app ou tocar " +
                "em algo que carrega, em vez de tirar vários prints.",
            esquema(
                "texto" to prop("string", "Texto (ou parte dele) a esperar."),
                "sumir" to prop("boolean", "Espera o texto SUMIR."),
                "ms" to prop("integer", "Tempo máximo (padrão 10000, máximo 50000)."),
                obrigatorios = listOf("texto"),
            ),
        ) { a ->
            val alvo = a.getString("texto")
            val sumir = a.optBoolean("sumir")
            val limite = System.currentTimeMillis() + a.optLong("ms", 10_000).coerceIn(100, 50_000)
            while (true) {
                val achado = Tela.todos().firstOrNull { n ->
                    n.isVisibleToUser && (n.text?.toString()?.contains(alvo, true) == true ||
                        n.contentDescription?.toString()?.contains(alvo, true) == true)
                }
                if (sumir && achado == null) return@Ferramenta Resultado.texto("\"$alvo\" sumiu.")
                if (!sumir && achado != null) return@Ferramenta Resultado.texto("Apareceu: ${Tela.rotulo(achado)}.")
                if (System.currentTimeMillis() > limite) {
                    throw Falha(if (sumir) "\"$alvo\" continua na tela." else "\"$alvo\" não apareceu.")
                }
                Thread.sleep(300)
            }
            @Suppress("UNREACHABLE_CODE") Resultado.texto("")
        },

        Ferramenta(
            "midia",
            "O que está tocando agora (app, música, artista, TOCANDO/pausado, posição, volume). O Claude não ouve o carro: " +
                "é assim que se confere se o Spotify está tocando de verdade. Com {acao} controla: tocar, pausar, proxima, anterior, parar.",
            esquema(
                "acao" to prop("string", "tocar | pausar | proxima | anterior | parar"),
                "pacote" to prop("string", "Qual app controlar (padrão: o primeiro da lista)."),
            ),
        ) { a ->
            if (a.has("acao")) {
                val r = Notificacoes.controlar(a.optString("pacote").ifEmpty { null }, a.getString("acao"))
                Thread.sleep(800)
                Resultado.texto(r + "\n\n" + Notificacoes.descreverMidia())
            } else {
                Resultado.texto(Notificacoes.descreverMidia())
            }
        },

        Ferramenta(
            "notificacoes",
            "Lista as notificações abertas na central.",
            esquema(),
        ) { Resultado.texto(Notificacoes.descreverNotificacoes()) },

        Ferramenta(
            "instalar_apk",
            "Baixa um APK de uma {url} e instala, tocando em \"Instalar\" sozinho. Para um APK que está no Mac, rode " +
                "`node scripts/carro-apk.mjs <arquivo.apk>`, que manda o arquivo pelo relay. Instalar o Eclipse por cima do " +
                "instalado só funciona com a MESMA assinatura (release com release).",
            esquema("url" to prop("string", "URL do APK."), obrigatorios = listOf("url")),
            timeoutMs = 300_000,
        ) { a -> Instalador.deUrl(a.getString("url")) },

        Ferramenta(
            "abrir_config",
            "Abre uma tela de configuração do Android: acessibilidade, notificacoes, instalar_apps (permitir que o Mãos " +
                "instale), app (detalhes de {pacote}: forçar parada, permissões, armazenamento), wifi, bluetooth, geral.",
            esquema(
                "tela" to prop("string", "acessibilidade | notificacoes | instalar_apps | app | wifi | bluetooth | geral"),
                "pacote" to prop("string", "Para tela=app."),
                obrigatorios = listOf("tela"),
            ),
        ) { a ->
            val ctx = Conexao.app
            val i = when (a.getString("tela")) {
                "acessibilidade" -> Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
                "notificacoes" -> Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
                "instalar_apps" -> Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${ctx.packageName}"))
                "app" -> Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${a.optString("pacote", ctx.packageName)}"))
                "wifi" -> Intent(Settings.ACTION_WIFI_SETTINGS)
                "bluetooth" -> Intent(Settings.ACTION_BLUETOOTH_SETTINGS)
                "geral" -> Intent(Settings.ACTION_SETTINGS)
                else -> throw Falha("Tela desconhecida: ${a.getString("tela")}")
            }
            try {
                ctx.startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            } catch (e: android.content.ActivityNotFoundException) {
                throw Falha("Esta central não tem a tela \"${a.getString("tela")}\" (a FYT desligou).")
            }
            Resultado.texto("Abri ${a.getString("tela")}.")
        },

        Ferramenta(
            "info",
            "Sobre a central e o Mãos: modelo, Android, tela, app em primeiro plano e quais permissões estão ligadas.",
            esquema(),
        ) { Resultado.texto(resumo().toString(2)) },
    )

    // --- apoio ----------------------------------------------------------------

    fun resumo(): JSONObject {
        val ctx = Conexao.app
        val (w, h) = tamanhoTela()
        val frente = try {
            Acessibilidade.instancia?.rootInActiveWindow?.packageName?.toString()
        } catch (e: Exception) { null }
        return JSONObject().apply {
            put("aparelho", "${Build.MANUFACTURER} ${Build.MODEL}")
            put("android", "${Build.VERSION.RELEASE} (sdk ${Build.VERSION.SDK_INT})")
            put("tela", "${w}x$h @ ${densidade()} dpi")
            put("maos", BuildConfig.VERSION_NAME)
            put("primeiro_plano", frente ?: JSONObject.NULL)
            put("acessibilidade", Acessibilidade.instancia != null)
            put("notificacoes", Notificacoes.instancia != null)
            put("print", Captura.ativa)
            put("pode_instalar", ctx.packageManager.canRequestPackageInstalls())
        }
    }

    fun tamanhoTela(): Pair<Int, Int> {
        val m = metricas()
        return m.widthPixels to m.heightPixels
    }

    fun densidade(): Int = metricas().densityDpi

    private fun metricas(): DisplayMetrics {
        val wm = Conexao.app.getSystemService(WindowManager::class.java)
        return DisplayMetrics().also {
            @Suppress("DEPRECATION")
            wm.defaultDisplay.getRealMetrics(it)
        }
    }

    private fun ponto(a: JSONObject): Triple<Float, Float, String> {
        if (a.has("x") && a.has("y")) {
            val x = a.getDouble("x").toFloat()
            val y = a.getDouble("y").toFloat()
            return Triple(x, y, "em ($x,$y)")
        }
        val no = Tela.achar(a) ?: throw Falha("Diga onde: {x,y}, {no}, {texto} ou {id}.")
        val (x, y) = Tela.centro(no)
        return Triple(x, y, Tela.rotulo(no))
    }

    private fun linha(x1: Float, y1: Float, x2: Float, y2: Float) = Path().apply {
        moveTo(x1, y1)
        lineTo(x2, y2)
    }

    private fun traco(x1: Float, y1: Float, x2: Float, y2: Float, ms: Long) =
        GestureDescription.Builder()
            .addStroke(GestureDescription.StrokeDescription(linha(x1, y1, x2, y2), 0, ms))
            .build()

    private fun gesto(g: GestureDescription) {
        val s = Acessibilidade.exigir()
        val fim = CountDownLatch(1)
        var completo = false
        val ok = s.dispatchGesture(g, object : AccessibilityService.GestureResultCallback() {
            override fun onCompleted(d: GestureDescription) {
                completo = true
                fim.countDown()
            }

            override fun onCancelled(d: GestureDescription) {
                fim.countDown()
            }
        }, null)
        if (!ok) throw Falha("O Android recusou o gesto.")
        fim.await(15, TimeUnit.SECONDS)
        if (!completo) throw Falha("O gesto foi interrompido (outro toque ao mesmo tempo?).")
        // Dá tempo para a tela reagir antes do próximo `print`/`tela`.
        Thread.sleep(250)
    }

    private fun prop(tipo: String, descricao: String) =
        JSONObject().put("type", tipo).apply { if (descricao.isNotEmpty()) put("description", descricao) }

    private fun esquema(vararg props: Pair<String, JSONObject>, obrigatorios: List<String> = emptyList()) =
        JSONObject().apply {
            put("type", "object")
            put("properties", JSONObject().apply { props.forEach { (k, v) -> put(k, v) } })
            if (obrigatorios.isNotEmpty()) put("required", JSONArray(obrigatorios))
        }
}
