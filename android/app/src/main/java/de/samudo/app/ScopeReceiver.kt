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
 * Tasker chooses the scope on this phone (ADR 0043 §8): a broadcast with
 * the package `de.samudo.app`, the action [ACTION] and the extra `scope`,
 * a scope's title or id; empty for none, `auto` to follow the schedule.
 * It is an event like a choice by hand (§7). An unknown scope changes
 * nothing and says so in a notification. Open like the ring's receiver:
 * another app can only switch what this phone shows.
 */
class ScopeReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION) return
        val word = intent.getStringExtra(SCOPE).orEmpty()
        val done = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val now = Store.get(context).chooseScopeByWord(word)
                Log.i("samu", "scope from Tasker: $now")
                Widgets.refresh(context)
                redrawApp()
            } catch (e: Exception) {
                Log.w("samu", "scope from Tasker: $e")
                tell(context, word, e.message ?: e.toString())
            } finally {
                done.finish()
            }
        }
    }

    private fun tell(context: Context, word: String, reason: String) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Scope", NotificationManager.IMPORTANCE_LOW),
        )
        val notification = android.app.Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.widget_ring)
            .setContentTitle(tr("Scope „{word}“ nicht gewählt", "word" to word))
            .setContentText(reason)
            .setShowWhen(true)
            .build()
        manager.notify(CHANNEL, 1, notification)
    }

    companion object {
        const val ACTION = "de.samudo.app.action.SCOPE"
        const val SCOPE = "scope"
        private const val CHANNEL = "scope"
    }
}
