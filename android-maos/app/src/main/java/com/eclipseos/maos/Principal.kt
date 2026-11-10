package com.eclipseos.maos

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.TypedValue
import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

/**
 * A única tela do app: o que está ligado e um botão para cada coisa que falta.
 * O dono abre isto uma vez, liga as permissões, e não precisa voltar.
 */
class Principal : Activity() {
    private val atualizar = Handler(Looper.getMainLooper())
    private lateinit var lista: LinearLayout

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        lista = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(48, 32, 48, 32)
        }
        setContentView(android.widget.ScrollView(this).apply { addView(lista) })
    }

    override fun onResume() {
        super.onResume()
        desenhar()
    }

    override fun onPause() {
        atualizar.removeCallbacksAndMessages(null)
        super.onPause()
    }

    private fun desenhar() {
        lista.removeAllViews()
        titulo("Eclipse Mãos")
        texto("Deixa o Claude ver e tocar na tela da central, de longe. Não fala e não muda o seu toque.")

        val acess = Acessibilidade.instancia != null
        linha(
            "Acessibilidade", acess,
            if (acess) null else "Ligar" to { abrir(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) },
            "Em Acessibilidade, toque em \"Eclipse Mãos\" e ligue. É o que deixa o Claude tocar na tela.",
        )
        val conectado = Conexao.conectado
        linha(
            "Conexão com o Claude", conectado, null,
            when {
                conectado -> "Conectado."
                !acess -> "Liga sozinha quando a acessibilidade estiver ligada."
                else -> "Tentando… ${Conexao.ultimoErro ?: ""}"
            },
        )
        val notif = Notificacoes.instancia != null
        linha(
            "Ver o que está tocando", notif,
            if (notif) null else "Ligar" to { abrir(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)) },
            "Acesso a notificações. É como o Claude confere se o Spotify está tocando.",
        )
        linha(
            "Print da tela", Captura.ativa,
            if (Captura.ativa) null else "Permitir" to { startActivity(Intent(this, PedirCaptura::class.java)) },
            "Depois de ligar a central o Android pergunta de novo; com a acessibilidade ligada, o Mãos aceita sozinho.",
        )
        val instalar = packageManager.canRequestPackageInstalls()
        linha(
            "Instalar apps", instalar,
            if (instalar) null else "Permitir" to {
                abrir(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName")))
            },
            "Para o Claude instalar a versão nova do Eclipse sem ninguém no carro.",
        )
        atualizar.postDelayed({ desenhar() }, 2000)
    }

    private fun abrir(i: Intent) {
        try {
            startActivity(i)
        } catch (e: ActivityNotFoundException) {
            texto("Esta central não tem essa tela de configuração.")
        }
    }

    private fun titulo(t: String) = lista.addView(TextView(this).apply {
        text = t
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 28f)
        setPadding(0, 0, 0, 16)
    })

    private fun texto(t: String) = lista.addView(TextView(this).apply {
        text = t
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
        alpha = 0.7f
        setPadding(0, 0, 0, 24)
    })

    private fun linha(nome: String, ok: Boolean, botao: Pair<String, () -> Unit>?, detalhe: String) {
        val l = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, 12, 0, 12)
        }
        val textos = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        textos.addView(TextView(this).apply {
            text = (if (ok) "✓  " else "○  ") + nome
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
        })
        textos.addView(TextView(this).apply {
            text = detalhe
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
            alpha = 0.6f
        })
        l.addView(textos, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        if (botao != null) {
            l.addView(Button(this).apply {
                text = botao.first
                setOnClickListener { botao.second() }
            })
        }
        lista.addView(l)
    }
}
