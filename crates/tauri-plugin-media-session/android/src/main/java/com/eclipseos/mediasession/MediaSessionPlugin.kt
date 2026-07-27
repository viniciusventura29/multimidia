package com.eclipseos.mediasession

// ATENÇÃO: nada neste arquivo foi compilado. Escrito contra a API documentada
// do Android (developer.android.com) e do Tauri v2 para plugins Android, mas
// sem SDK, `adb` nem Java neste Mac para de fato rodar `javac`/`kotlinc` sobre
// ele. O primeiro `npx tauri android dev` com o SDK instalado é quem prova.

import android.app.Activity
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/** O pacote do Spotify, para preferir a sessão dele quando há mais de uma ativa. */
private const val PACOTE_SPOTIFY = "com.spotify.music"

@TauriPlugin
class MediaSessionPlugin(private val activity: Activity) : Plugin(activity) {

    private val gerenciador: MediaSessionManager
        get() = activity.getSystemService(Context.MEDIA_SESSION_SERVICE) as MediaSessionManager

    private val componenteListener: ComponentName
        get() = ComponentName(activity, EclipseNotificationListener::class.java)

    /**
     * A sessão a escolher quando há mais de uma ativa (Spotify e um navegador
     * tocando um vídeo, por exemplo): a do Spotify primeiro; senão, a que
     * estiver tocando de verdade; senão, a primeira disponível.
     */
    private fun sessaoAtiva(): MediaController? {
        val sessoes = gerenciador.getActiveSessions(componenteListener)
        return sessoes.find { it.packageName == PACOTE_SPOTIFY }
            ?: sessoes.find { it.playbackState?.state == PlaybackState.STATE_PLAYING }
            ?: sessoes.firstOrNull()
    }

    @Command
    fun now_playing(invoke: Invoke) {
        try {
            val sessao = sessaoAtiva()
            if (sessao == null) {
                // Nada tocando em lugar nenhum é um estado normal, não erro —
                // o mesmo "None" que a fonte do Spotify Web API já devolvia.
                invoke.resolve(null)
                return
            }

            val metadados = sessao.metadata
            val estado = sessao.playbackState

            val resultado = JSObject()
            resultado.put("track", metadados?.getString(MediaMetadata.METADATA_KEY_TITLE))
            resultado.put("artist", metadados?.getString(MediaMetadata.METADATA_KEY_ARTIST))
            resultado.put("albumArtUri", metadados?.getString(MediaMetadata.METADATA_KEY_ALBUM_ART_URI))
            resultado.put("isPlaying", estado?.state == PlaybackState.STATE_PLAYING)
            resultado.put("positionMs", estado?.position)
            resultado.put(
                "durationMs",
                metadados?.getLong(MediaMetadata.METADATA_KEY_DURATION)?.takeIf { it > 0 },
            )
            resultado.put("packageName", sessao.packageName)

            invoke.resolve(resultado)
        } catch (e: SecurityException) {
            // Sem "acesso a notificações" concedido, é isto que o Android lança.
            invoke.reject("sem acesso a notificações")
        }
    }

    @Command
    fun play(invoke: Invoke) {
        sessaoAtiva()?.transportControls?.play()
        invoke.resolve()
    }

    @Command
    fun pause(invoke: Invoke) {
        sessaoAtiva()?.transportControls?.pause()
        invoke.resolve()
    }

    @Command
    fun next(invoke: Invoke) {
        sessaoAtiva()?.transportControls?.skipToNext()
        invoke.resolve()
    }

    @Command
    fun previous(invoke: Invoke) {
        sessaoAtiva()?.transportControls?.skipToPrevious()
        invoke.resolve()
    }

    @Command
    fun has_notification_access(invoke: Invoke) {
        val concedido = NotificationManagerCompat
            .getEnabledListenerPackages(activity)
            .contains(activity.packageName)

        val resultado = JSObject()
        resultado.put("value", concedido)
        invoke.resolve(resultado)
    }

    /**
     * Abre a tela de Ajustes onde o usuário concede o acesso.
     *
     * Não dá pra conceder isso programaticamente — é assim de propósito, para
     * um app não poder se autoconceder acesso a todas as notificações do
     * aparelho sem o usuário ver e decidir.
     */
    @Command
    fun request_notification_access(invoke: Invoke) {
        val intent = Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        activity.startActivity(intent)
        invoke.resolve()
    }
}
