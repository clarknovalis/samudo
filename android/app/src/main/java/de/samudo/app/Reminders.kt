// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.Manifest
import android.app.AlarmManager
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
import de.samudo.app.mobile.Notice
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * Reminders on the phone (ADR 0030 §5, §6): one alarm, the next one
 * `samu-mobile` names. When it goes off the app shows what `fire` returns
 * and sets the next. What reminds when is decided in samu-core, not here.
 */
object Reminders {
    private const val CHANNEL = "reminders"
    private const val ALARM = 1

    /** Set the one alarm anew: after a sync, a change, opening, a restart. */
    fun arm(context: Context) {
        val context = context.applicationContext
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val samu = Store.get(context)
                val manager = context.getSystemService(AlarmManager::class.java)
                val pending = alarmIntent(context)
                val next = if (samu.status().signedIn) samu.nextAlarm(System.currentTimeMillis()) else null
                if (next == null) {
                    manager.cancel(pending)
                    return@launch
                }
                val exact = Build.VERSION.SDK_INT < Build.VERSION_CODES.S || manager.canScheduleExactAlarms()
                // Without leave for exact alarms it comes a little later, but it comes.
                if (exact) {
                    manager.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next, pending)
                } else {
                    manager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next, pending)
                }
                Log.i("samu", "reminder alarm at $next (exact: $exact)")
            } catch (e: Exception) {
                Log.w("samu", "reminder arm: $e")
            }
        }
    }

    private fun alarmIntent(context: Context): PendingIntent =
        PendingIntent.getBroadcast(
            context,
            ALARM,
            Intent(context, AlarmReceiver::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

    fun channel(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, tr("Erinnerungen"), NotificationManager.IMPORTANCE_HIGH),
        )
    }

    /**
     * One notification per task or project, with Erledigt, Später and
     * Morgen; per occurrence of an appointment with its time and place and
     * only Später (ADR 0101 §7).
     */
    fun show(context: Context, notices: List<Notice>) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            Log.i("samu", "reminders: ${notices.size} not shown, no leave to notify")
            return
        }
        channel(context)
        val manager = context.getSystemService(NotificationManager::class.java)
        for (notice in notices) {
            val tag = notice.key
            val appointment = notice.appointment
            val open = PendingIntent.getActivity(
                context,
                tag.hashCode(),
                Intent(context, MainActivity::class.java)
                    .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                    .apply {
                        if (notice.isTask) data = Uri.parse("$WIDGET_SCHEME:$TASK/$tag")
                        if (appointment != null) data = Uri.parse("$WIDGET_SCHEME:$APPOINTMENT/$tag")
                    },
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val builder = android.app.Notification.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.widget_ring)
                .setContentTitle(notice.title)
                .setContentText(appointment?.let { Words.appointmentNotice(it) } ?: Words.reminded(notice))
                .setWhen(notice.atMs)
                .setShowWhen(true)
                .setAutoCancel(true)
                .setCategory(android.app.Notification.CATEGORY_REMINDER)
                .setContentIntent(open)
            if (notice.isTask) builder.addAction(action(context, NoticeAction.DONE, tag, tr("Erledigt")))
            builder.addAction(action(context, NoticeAction.LATER, tag, tr("Später")))
            if (appointment == null) builder.addAction(action(context, NoticeAction.TOMORROW, tag, tr("Morgen")))
            manager.notify(tag, 0, builder.build())
        }
    }

    fun dismiss(context: Context, owner: String) {
        context.getSystemService(NotificationManager::class.java).cancel(owner, 0)
    }

    private fun action(context: Context, what: String, owner: String, label: String): android.app.Notification.Action {
        val intent = Intent(context, NoticeAction::class.java)
            .setAction(what)
            // A data URI of its own, so the system keeps the intents apart.
            .setData(Uri.parse("samu-reminder:$what/$owner"))
            .putExtra(NoticeAction.OWNER, owner)
        val pending = PendingIntent.getBroadcast(
            context,
            0,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return android.app.Notification.Action.Builder(null, label, pending).build()
    }
}

/** The alarm: show what is due, set the next. */
class AlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val done = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val notices = Store.get(context).fire(System.currentTimeMillis())
                Log.i("samu", "reminder fired: ${notices.size} to show")
                Reminders.show(context, notices)
            } catch (e: Exception) {
                Log.w("samu", "reminder fire: $e")
            } finally {
                Reminders.arm(context)
                done.finish()
            }
        }
    }
}

/** After a restart or an update the alarm is gone; set it again. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED -> {
                Reminders.arm(context)
                Midnight.arm(context)
            }
        }
    }
}

/** Erledigt goes through the outbox; Später and Morgen stay on the phone. */
class NoticeAction : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val owner = intent.getStringExtra(OWNER) ?: return
        Reminders.dismiss(context, owner)
        val done = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val samu = Store.get(context)
                val now = System.currentTimeMillis()
                when (intent.action) {
                    DONE -> {
                        samu.complete(owner)
                        Sync.afterChange(context)
                        Widgets.refresh(context)
                        Clocking.refresh(context)
                    }
                    LATER -> samu.snooze(owner, 60, now)
                    TOMORROW -> samu.snoozeTomorrow(owner, now)
                }
            } catch (e: Exception) {
                Log.w("samu", "reminder action: $e")
            } finally {
                Reminders.arm(context)
                done.finish()
            }
        }
    }

    companion object {
        const val OWNER = "owner"
        const val DONE = "de.samudo.app.reminder.DONE"
        const val LATER = "de.samudo.app.reminder.LATER"
        const val TOMORROW = "de.samudo.app.reminder.TOMORROW"
    }
}
