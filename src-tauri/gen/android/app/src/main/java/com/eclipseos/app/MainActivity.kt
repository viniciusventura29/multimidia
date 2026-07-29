package com.eclipseos.app

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // `enableEdgeToEdge` faz o app desenhar por baixo das barras do sistema — e
    // sozinho isso escondia o rodapé do painel (a barra do "tocando agora", com
    // os controles) atrás da barra de navegação do Android. A saída não é voltar
    // atrás: num painel de carro as barras do Android não deveriam aparecer.
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    esconderBarrasDoSistema()
  }

  /**
   * Redesconde depois de perder e retomar o foco: diálogos do sistema (permissão
   * de localização, escolha de app) trazem as barras de volta, e sem isto o
   * painel voltaria com a faixa de baixo comida.
   */
  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus) esconderBarrasDoSistema()
  }

  /**
   * Tela cheia de verdade. `BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE` mantém as
   * barras alcançáveis por um arrasto da borda — no celular de teste ainda é
   * preciso sair do app, e na head unit não incomoda.
   */
  private fun esconderBarrasDoSistema() {
    WindowCompat.getInsetsController(window, window.decorView).apply {
      systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
      hide(WindowInsetsCompat.Type.systemBars())
    }
  }
}
