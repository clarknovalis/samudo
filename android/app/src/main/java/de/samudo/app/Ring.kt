// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * The ring's net without Tailscale (ADR 0032): Tasker sends a broadcast
 * with the package `de.samudo.app`, the action [ACTION], the extra `text`
 * and, if it has one, `recorded_at` in Unix milliseconds. The task comes
 * from `samu-mobile` like the hook's and waits in the outbox. If it cannot
 * be filed, a notification keeps the text.
 */
class RingReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION) return
        val text = intent.getStringExtra(TEXT).orEmpty()
        val recordedAt = recordedAt(intent)
        val done = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val id = Store.get(context).captureRing(text, recordedAt)
                Log.i("samu", "ring: filed $id")
                Sync.afterChange(context)
                Widgets.refresh(context)
                redrawApp()
            } catch (e: Exception) {
                Log.w("samu", "ring: not filed: $e")
                keep(context, text, e.message ?: e.toString())
            } finally {
                done.finish()
            }
        }
    }

    /** Tasker sends numbers as text unless told otherwise; take both. */
    private fun recordedAt(intent: Intent): Long? {
        if (!intent.hasExtra(RECORDED_AT)) return null
        intent.getStringExtra(RECORDED_AT)?.let { return it.trim().toLongOrNull() }
        val long = intent.getLongExtra(RECORDED_AT, Long.MIN_VALUE)
        if (long != Long.MIN_VALUE) return long
        val int = intent.getIntExtra(RECORDED_AT, Int.MIN_VALUE)
        return if (int != Int.MIN_VALUE) int.toLong() else null
    }

    /** Nothing lost silently (ADR 0032 §3): the text in a notification. */
    private fun keep(context: Context, text: String, reason: String) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            Log.w("samu", "ring: no leave to notify; the text was: $text")
            return
        }
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Ring", NotificationManager.IMPORTANCE_DEFAULT),
        )
        val shown = text.ifBlank { tr("(ohne Text)") }
        val notification = android.app.Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.widget_ring)
            .setContentTitle(tr("Aufnahme vom Ring nicht abgelegt"))
            .setContentText(shown)
            .setStyle(android.app.Notification.BigTextStyle().bigText("$shown\n\n$reason"))
            .setShowWhen(true)
            .build()
        manager.notify(CHANNEL, System.currentTimeMillis().toInt(), notification)
    }

    companion object {
        const val ACTION = "de.samudo.app.action.RING"
        const val TEXT = "text"
        const val RECORDED_AT = "recorded_at"
        private const val CHANNEL = "ring"
    }
}
