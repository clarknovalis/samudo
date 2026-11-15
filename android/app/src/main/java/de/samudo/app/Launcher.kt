package de.samudo.app

import android.app.BroadcastOptions
import android.content.Context
import android.content.Intent
import android.os.Build
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Tells minlauncher, the home screen on the phone, how the inbox and the
 * day stand, so that it can show them around its clock without a widget.
 *
 * An explicit broadcast to that one package. The launcher takes it only
 * from here: from Android 14 on the broadcast carries who sent it
 * ([BroadcastOptions.setShareIdentityEnabled]), and the launcher checks
 * that. Nothing in it but four counts.
 */
object Launcher {
    private const val PACKAGE = "org.example.launcher"
    private const val ACTION = "org.example.launcher.action.GLANCE"

    suspend fun tell(context: Context) {
        val glance = withContext(Dispatchers.IO) {
            try {
                val samu = Store.get(context)
                if (samu.status().signedIn) samu.glance() else null
            } catch (e: Exception) {
                Log.w("samu", "launcher glance: $e")
                null
            }
        }
        val intent = Intent(ACTION).setPackage(PACKAGE)
            .putExtra("source", "samu")
            .putExtra("stamp", System.currentTimeMillis())
        if (glance != null) {
            intent.putExtra("inbox", glance.inbox.toInt())
                .putExtra("overdue", glance.overdue.toInt())
                .putExtra("today_open", glance.todayOpen.toInt())
                .putExtra("today_done", glance.todayDone.toInt())
        }
        // Signed out: only source and stamp, which clears the launcher's dots.
        try {
            if (Build.VERSION.SDK_INT >= 34) {
                val options = BroadcastOptions.makeBasic().setShareIdentityEnabled(true)
                context.sendBroadcast(intent, null, options.toBundle())
            } else {
                context.sendBroadcast(intent)
            }
        } catch (e: Exception) {
            Log.w("samu", "launcher tell: $e")
        }
    }
}
