package com.eclipseos.mediasession

import android.service.notification.NotificationListenerService

/**
 * Existe só para o Android ter alguém a quem conceder "acesso a notificações".
 *
 * O `MediaSessionManager.getActiveSessions()` exige que o chamador aponte para
 * um `NotificationListenerService` habilitado pelo usuário — mesmo que a gente
 * não leia o conteúdo de nenhuma notificação. Por isso os métodos de
 * `onNotificationPosted`/`onNotificationRemoved` ficam vazios de propósito: o
 * papel desta classe é só existir e estar declarada no manifest.
 */
class EclipseNotificationListener : NotificationListenerService()
