package com.eclipseos.maos

import android.accessibilityservice.AccessibilityService
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo

/**
 * O serviço de acessibilidade é a única porta que o Android dá, sem root e sem
 * ADB, para um app ler a tela de OUTROS apps e tocar nela. A central da FYT
 * trancou o ADB, mas deixou esta porta aberta (testado com o TalkBack em
 * 09/11/2026).
 *
 * Além de servir as ferramentas, o serviço tem um "piloto": quando o próprio
 * Mãos pede algo que o Android só concede com um toque humano (o "Iniciar
 * agora" do print, o "Instalar" do instalador), o piloto toca por ele. Só age
 * enquanto um desses pedidos está armado, e só nas janelas do sistema.
 */
class Acessibilidade : AccessibilityService() {

    override fun onServiceConnected() {
        instancia = this
        Log.i("maos", "acessibilidade ligada")
        Conexao.ligar(this)
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent) {
        val pacote = event.packageName?.toString() ?: return
        val agora = System.currentTimeMillis()
        if (agora < consentirCapturaAte && pacote == "com.android.systemui") {
            if (clicarBotao(setOf("android:id/button1"), setOf("iniciar agora", "começar agora", "start now"))) {
                consentirCapturaAte = 0
            }
        }
        if (agora < instalarAte && pacote.contains("packageinstaller")) {
            clicarBotao(
                setOf("ok_button", "android:id/button1"),
                setOf("instalar", "atualizar", "install", "update"),
            )
        }
    }

    override fun onInterrupt() {}

    override fun onDestroy() {
        if (instancia === this) instancia = null
        Conexao.desligar()
        super.onDestroy()
    }

    /** Procura na janela ativa um botão por id ou texto e clica, se habilitado. */
    private fun clicarBotao(ids: Set<String>, textos: Set<String>): Boolean {
        val raiz = rootInActiveWindow ?: return false
        val achado = Tela.percorrer(raiz).firstOrNull { no ->
            val id = no.viewIdResourceName ?: ""
            val texto = no.text?.toString()?.trim()?.lowercase() ?: ""
            no.isEnabled && no.isVisibleToUser &&
                (ids.any { id == it || id.endsWith(":id/$it") } || texto in textos)
        } ?: return false
        Log.i("maos", "piloto: clicando ${achado.viewIdResourceName} \"${achado.text}\"")
        return achado.performAction(AccessibilityNodeInfo.ACTION_CLICK)
    }

    companion object {
        @Volatile var instancia: Acessibilidade? = null
            private set

        @Volatile var consentirCapturaAte = 0L
        @Volatile var instalarAte = 0L

        fun exigir(): Acessibilidade = instancia
            ?: throw Falha("O serviço de acessibilidade do Eclipse Mãos está desligado.")
    }
}
