package com.eclipseos.maos

import android.app.Notification
import android.content.ComponentName
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.service.notification.NotificationListenerService

/**
 * Acesso a notificações. Serve para duas coisas:
 *  - ler as sessões de mídia ativas (o Claude não ouve o carro; é assim que
 *    ele confere que o Spotify está TOCANDO, e em que segundo);
 *  - ler as notificações abertas.
 *
 * O Android só entrega a lista de sessões de mídia a quem tem um listener de
 * notificações ligado, por isso as duas coisas moram juntas.
 */
class Notificacoes : NotificationListenerService() {
    override fun onListenerConnected() {
        instancia = this
    }

    override fun onListenerDisconnected() {
        if (instancia === this) instancia = null
    }

    companion object {
        @Volatile var instancia: Notificacoes? = null
            private set

        private fun exigir(): Notificacoes = instancia ?: throw Falha(
            "O acesso a notificações do Eclipse Mãos está desligado. Chame `abrir_config {tela: \"notificacoes\"}` e ligue o Eclipse Mãos.",
        )

        fun sessoes(): List<MediaController> {
            val s = exigir()
            val msm = s.getSystemService(MediaSessionManager::class.java)
            return msm.getActiveSessions(ComponentName(s, Notificacoes::class.java))
        }

        fun descreverMidia(): String {
            val lista = sessoes()
            if (lista.isEmpty()) return "Nenhuma sessão de mídia ativa."
            return lista.joinToString("\n\n") { c ->
                val est = c.playbackState
                val meta = c.metadata
                buildString {
                    append(c.packageName).append(": ").append(estado(est?.state))
                    meta?.getString(MediaMetadata.METADATA_KEY_TITLE)?.let { append("\n  música: ").append(it) }
                    meta?.getString(MediaMetadata.METADATA_KEY_ARTIST)?.let { append("\n  artista: ").append(it) }
                    meta?.getString(MediaMetadata.METADATA_KEY_ALBUM)?.let { append("\n  álbum: ").append(it) }
                    if (est != null) {
                        val dur = meta?.getLong(MediaMetadata.METADATA_KEY_DURATION) ?: 0
                        append("\n  posição: ").append(mmss(posicaoAgora(est)))
                        if (dur > 0) append(" de ").append(mmss(dur))
                    }
                    c.playbackInfo?.let { append("\n  volume: ").append(it.currentVolume).append('/').append(it.maxVolume) }
                }
            }
        }

        fun controlar(pacote: String?, acao: String): String {
            val lista = sessoes()
            val c = (if (pacote != null) lista.firstOrNull { it.packageName == pacote } else lista.firstOrNull())
                ?: throw Falha(if (pacote != null) "Nenhuma sessão de mídia de $pacote." else "Nenhuma sessão de mídia ativa.")
            val t = c.transportControls
            when (acao) {
                "tocar" -> t.play()
                "pausar" -> t.pause()
                "proxima" -> t.skipToNext()
                "anterior" -> t.skipToPrevious()
                "parar" -> t.stop()
                else -> throw Falha("Ação de mídia desconhecida: $acao")
            }
            return "Mandei \"$acao\" para ${c.packageName}."
        }

        fun descreverNotificacoes(): String {
            val lista = exigir().activeNotifications ?: return "Nenhuma notificação."
            if (lista.isEmpty()) return "Nenhuma notificação."
            return lista.joinToString("\n") { sbn ->
                val e = sbn.notification.extras
                val titulo = e.getCharSequence(Notification.EXTRA_TITLE)
                val texto = e.getCharSequence(Notification.EXTRA_TEXT)
                "${sbn.packageName}: ${titulo ?: ""}${if (texto != null) " — $texto" else ""}"
            }
        }

        private fun posicaoAgora(est: PlaybackState): Long {
            if (est.state != PlaybackState.STATE_PLAYING) return est.position
            val passou = android.os.SystemClock.elapsedRealtime() - est.lastPositionUpdateTime
            return est.position + (passou * est.playbackSpeed).toLong()
        }

        private fun mmss(ms: Long): String {
            val s = ms / 1000
            return "%d:%02d".format(s / 60, s % 60)
        }

        private fun estado(s: Int?) = when (s) {
            PlaybackState.STATE_PLAYING -> "TOCANDO"
            PlaybackState.STATE_PAUSED -> "pausado"
            PlaybackState.STATE_STOPPED -> "parado"
            PlaybackState.STATE_BUFFERING -> "carregando"
            PlaybackState.STATE_CONNECTING -> "conectando"
            PlaybackState.STATE_ERROR -> "ERRO"
            PlaybackState.STATE_NONE, null -> "sem estado"
            else -> "estado $s"
        }
    }
}
