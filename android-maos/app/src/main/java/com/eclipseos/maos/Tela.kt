package com.eclipseos.maos

import android.graphics.Rect
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo

/**
 * A árvore de elementos de todas as janelas, em texto que o Claude lê bem.
 *
 * Cada elemento relevante ganha um número (`#12`) que vale até o próximo
 * `tela`: `tocar {no: 12}` usa o elemento guardado aqui. O WebView do Eclipse
 * também entra, porque o Chromium expõe o DOM como nós de acessibilidade
 * quando um serviço pede o conteúdo das janelas.
 */
object Tela {
    private const val MAX_NOS = 1500
    private const val MAX_TEXTO = 80

    @Volatile private var ultima: List<AccessibilityNodeInfo> = emptyList()

    fun raizes(): List<Pair<AccessibilityWindowInfo?, AccessibilityNodeInfo>> {
        val s = Acessibilidade.exigir()
        val janelas = s.windows
        if (janelas.isNullOrEmpty()) return listOfNotNull(s.rootInActiveWindow?.let { null to it })
        return janelas.mapNotNull { j -> j.root?.let { j to it } }
    }

    fun percorrer(raiz: AccessibilityNodeInfo): Sequence<AccessibilityNodeInfo> = sequence {
        val pilha = ArrayDeque<AccessibilityNodeInfo>()
        pilha.addLast(raiz)
        while (pilha.isNotEmpty()) {
            val no = pilha.removeLast()
            yield(no)
            for (i in no.childCount - 1 downTo 0) no.getChild(i)?.let { pilha.addLast(it) }
        }
    }

    fun todos(): Sequence<AccessibilityNodeInfo> = raizes().asSequence().flatMap { percorrer(it.second) }

    fun descrever(): String {
        val nos = mutableListOf<AccessibilityNodeInfo>()
        val sb = StringBuilder()
        val (w, h) = Ferramentas.tamanhoTela()
        sb.append("Tela ${w}x$h. Coordenadas em pixels.\n")
        for ((janela, raiz) in raizes()) {
            sb.append("\n== ")
            sb.append(janela?.title ?: raiz.packageName ?: "?")
            sb.append(" (").append(raiz.packageName).append(")")
            if (janela != null) {
                sb.append(" tipo=").append(tipoJanela(janela.type))
                if (janela.isActive) sb.append(" ATIVA")
            }
            sb.append('\n')
            escrever(raiz, 0, nos, sb)
            if (nos.size >= MAX_NOS) {
                sb.append("… cortado em $MAX_NOS elementos\n")
                break
            }
        }
        ultima = nos
        return sb.toString()
    }

    private fun escrever(no: AccessibilityNodeInfo, nivel: Int, nos: MutableList<AccessibilityNodeInfo>, sb: StringBuilder) {
        if (nos.size >= MAX_NOS) return
        if (!no.isVisibleToUser) return
        var proximoNivel = nivel
        if (relevante(no)) {
            nos.add(no)
            val r = Rect().also { no.getBoundsInScreen(it) }
            sb.append("  ".repeat(nivel))
            sb.append('#').append(nos.size - 1).append(' ')
            sb.append(no.className?.toString()?.substringAfterLast('.') ?: "?")
            no.text?.takeIf { it.isNotBlank() }?.let { sb.append(" \"").append(curto(it)).append('"') }
            no.contentDescription?.takeIf { it.isNotBlank() && it != no.text }?.let {
                sb.append(" [").append(curto(it)).append(']')
            }
            no.viewIdResourceName?.let { sb.append(" id=").append(it.substringAfter(":id/")) }
            sb.append(" (").append(r.left).append(',').append(r.top).append(',')
                .append(r.right).append(',').append(r.bottom).append(')')
            if (no.isClickable) sb.append(" clicável")
            if (no.isLongClickable) sb.append(" toque-longo")
            if (no.isScrollable) sb.append(" rolável")
            if (no.isEditable) sb.append(" editável")
            if (no.isCheckable) sb.append(if (no.isChecked) " marcado" else " desmarcado")
            if (no.isFocused) sb.append(" foco")
            if (!no.isEnabled) sb.append(" desabilitado")
            sb.append('\n')
            proximoNivel = nivel + 1
        }
        for (i in 0 until no.childCount) no.getChild(i)?.let { escrever(it, proximoNivel, nos, sb) }
    }

    // Contêiner que só tem id (FrameLayout id=nav_buttons…) não é algo que se
    // toque nem se leia; mostrar todos multiplicava a lista por três.
    private fun relevante(no: AccessibilityNodeInfo) =
        !no.text.isNullOrBlank() || !no.contentDescription.isNullOrBlank() ||
            no.isClickable || no.isScrollable || no.isEditable || no.isCheckable

    private fun curto(t: CharSequence): String {
        val s = t.toString().replace('\n', ' ')
        return if (s.length > MAX_TEXTO) s.take(MAX_TEXTO) + "…" else s
    }

    private fun tipoJanela(t: Int) = when (t) {
        AccessibilityWindowInfo.TYPE_APPLICATION -> "app"
        AccessibilityWindowInfo.TYPE_SYSTEM -> "sistema"
        AccessibilityWindowInfo.TYPE_INPUT_METHOD -> "teclado"
        AccessibilityWindowInfo.TYPE_ACCESSIBILITY_OVERLAY -> "sobreposição"
        AccessibilityWindowInfo.TYPE_SPLIT_SCREEN_DIVIDER -> "divisor"
        else -> t.toString()
    }

    // --- achar um elemento ----------------------------------------------------

    /** Acha pelo número do último `tela`, por id ou por texto visível. */
    fun achar(args: org.json.JSONObject): AccessibilityNodeInfo? {
        if (args.has("no")) {
            val n = args.getInt("no")
            val no = ultima.getOrNull(n) ?: throw Falha("Não existe #$n no último `tela`. Chame `tela` de novo.")
            if (!no.refresh()) throw Falha("O #$n sumiu da tela desde o último `tela`. Chame `tela` de novo.")
            return no
        }
        if (args.has("id")) {
            val id = args.getString("id")
            return todos().firstOrNull { n ->
                n.isVisibleToUser && n.viewIdResourceName?.let { it == id || it.endsWith(":id/$id") } == true
            } ?: throw Falha("Nenhum elemento visível com id \"$id\".")
        }
        if (args.has("texto")) {
            val alvo = args.getString("texto").trim()
            val candidatos = todos().filter { n ->
                n.isVisibleToUser && (n.text?.toString()?.contains(alvo, true) == true ||
                    n.contentDescription?.toString()?.contains(alvo, true) == true)
            }.toList()
            if (candidatos.isEmpty()) throw Falha("Nenhum elemento visível com o texto \"$alvo\".")
            // Um campo de texto contém o que foi digitado nele: no teclado da
            // senha da fábrica, depois do primeiro "8", `texto: "8"` passava a
            // acertar o campo em vez da tecla. Quem é exatamente o texto e
            // clicável vem primeiro; campo editável, por último.
            fun exato(n: AccessibilityNodeInfo) =
                n.text?.toString()?.trim().equals(alvo, true) || n.contentDescription?.toString()?.trim().equals(alvo, true)
            return candidatos.sortedBy { n ->
                when {
                    n.isEditable -> 3
                    exato(n) && n.isClickable -> 0
                    exato(n) -> 1
                    else -> 2
                }
            }.first()
        }
        return null
    }

    /**
     * O campo onde se está digitando. A janela ativa vem primeiro: a tela
     * inicial atrás do app também tem uma busca, e o foco "de entrada" dela
     * continua existindo mesmo escondida.
     */
    fun campoComFoco(): AccessibilityNodeInfo? {
        val ordem = raizes().sortedByDescending { it.first?.isActive == true }
        for ((_, raiz) in ordem) {
            // Sem exigir "visível": o próprio teclado costuma cobrir o campo em
            // que se está digitando, e aí o Android o marca como invisível.
            val f = raiz.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            if (f != null && f.isEditable) return f
        }
        return ordem.asSequence().flatMap { percorrer(it.second) }.firstOrNull { it.isEditable && it.isFocused }
    }

    fun centro(no: AccessibilityNodeInfo): Pair<Float, Float> {
        val r = Rect().also { no.getBoundsInScreen(it) }
        return r.exactCenterX() to r.exactCenterY()
    }

    fun rotulo(no: AccessibilityNodeInfo): String {
        val r = Rect().also { no.getBoundsInScreen(it) }
        val nome = no.text ?: no.contentDescription ?: no.viewIdResourceName ?: no.className
        return "\"$nome\" em (${r.centerX()},${r.centerY()})"
    }
}
