// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.app.BroadcastOptions
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Tells the home screen how the inbox and the day stand, so that a
 * launcher that listens can show them around its clock without a widget.
 *
 * An explicit broadcast to the one app the user has set as home screen,
 * whichever that is; a launcher that wants the counts declares a receiver
 * for [ACTION]. From Android 14 on the broadcast carries who sent it
 * ([BroadcastOptions.setShareIdentityEnabled]), so the launcher can check
 * that it comes from here. Nothing in it but four counts.
 */
object Launcher {
    private const val ACTION = "de.samudo.app.action.GLANCE"

    /**
     * The package of the default home screen, or null while there is none:
     * with several launchers and no choice made the system answers with its
     * own chooser, and that is nobody to tell.
     */
    private fun home(context: Context): String? {
        val main = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        val found = context.packageManager
            .resolveActivity(main, PackageManager.MATCH_DEFAULT_ONLY)
            ?.activityInfo?.packageName
        return found?.takeIf { it != "android" && it != context.packageName }
    }

    suspend fun tell(context: Context) {
        val home = home(context) ?: return
        val glance = withContext(Dispatchers.IO) {
            try {
                val samu = Store.get(context)
                if (samu.status().signedIn) samu.glance() else null
            } catch (e: Exception) {
                Log.w("samu", "launcher glance: $e")
                null
            }
        }
        val intent = Intent(ACTION).setPackage(home)
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
