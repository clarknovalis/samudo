// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * The running clock as an ongoing notification (ADR 0046 §8): its title,
 * the time ticking from its start, and Stopp. No foreground service: the
 * notification only shows what `samu-mobile` says runs, and is drawn again
 * after every change and sync. That one clock runs and that it stops with
 * its task is decided in samu-core, not here.
 */
object Clocking {
    private const val CHANNEL = "clock"
    private const val TAG = "clock"

    fun refresh(context: Context) {
        val context = context.applicationContext
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val samu = Store.get(context)
                val on = if (samu.status().signedIn) samu.runningClock() else null
                val manager = context.getSystemService(NotificationManager::class.java)
                if (on == null) {
                    manager.cancel(TAG, 0)
                    return@launch
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                    context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
                ) {
                    return@launch
                }
                manager.createNotificationChannel(
                    NotificationChannel(CHANNEL, tr("Laufende Uhr"), NotificationManager.IMPORTANCE_LOW),
                )
                val open = PendingIntent.getActivity(
                    context,
                    TAG.hashCode(),
                    Intent(context, MainActivity::class.java)
                        .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                        .setData(Uri.parse("$WIDGET_SCHEME:$TASK/${on.ownerId}")),
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                )
                val stop = PendingIntent.getBroadcast(
                    context,
                    0,
                    Intent(context, ClockAction::class.java).setAction(ClockAction.STOP),
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                )
                val notification = Notification.Builder(context, CHANNEL)
                    .setSmallIcon(R.drawable.widget_ring)
                    .setContentTitle(on.title.ifBlank { tr("Uhr läuft") })
                    .setContentText(tr("Uhr läuft"))
                    .setWhen(on.startMs)
                    .setUsesChronometer(true)
                    .setShowWhen(true)
                    .setOngoing(true)
                    .setOnlyAlertOnce(true)
                    .setCategory(Notification.CATEGORY_STOPWATCH)
                    .setContentIntent(open)
                    .addAction(Notification.Action.Builder(null, tr("Stopp"), stop).build())
                    .build()
                manager.notify(TAG, 0, notification)
            } catch (e: Exception) {
                Log.w("samu", "clock notification: $e")
            }
        }
    }
}

/** Stopp in the notification: through the outbox, like everything. */
class ClockAction : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != STOP) return
        val done = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                Store.get(context).clockOut()
                Sync.afterChange(context)
            } catch (e: Exception) {
                Log.w("samu", "clock stop: $e")
            } finally {
                Clocking.refresh(context)
                done.finish()
            }
        }
    }

    companion object {
        const val STOP = "de.samudo.app.clock.STOP"
    }
}
