// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.content.Context
import android.util.Log
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import de.samudo.app.mobile.MobileException
import de.samudo.app.mobile.Samu
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.util.concurrent.TimeUnit

/**
 * The one database of the process, which the screens and the sync share
 * (ADR 0028 §3): two connections would each queue and push on their own.
 *
 * Each account on the phone has a folder of its own (ADR 0107 §1):
 * `files/samu` the first, as it always was, `files/samu-<n>` the others.
 * Which one is active stands in the preferences; only that one is open,
 * syncs and reminds (§3).
 */
object Store {
    private const val PREFS = "samu"
    private const val ACTIVE = "account"
    const val FIRST = "samu"
    private val SLOT = Regex("^samu(-\\d+)?$")

    @Volatile
    private var open: Samu? = null

    /** Opening also sets the language of the app (ADR 0105 §2). */
    @Synchronized
    fun get(context: Context): Samu =
        open ?: Samu.open(dir(context, active(context))).also {
            open = it
            I18n.init(context.applicationContext, it)
        }

    fun active(context: Context): String =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(ACTIVE, FIRST) ?: FIRST

    /** The accounts' folders, the first first; the active one always among them. */
    fun slots(context: Context): List<String> {
        val found = context.filesDir.listFiles().orEmpty()
            .filter { it.isDirectory && SLOT.matches(it.name) && it.resolve("samu.db").exists() }
            .map { it.name }
        return (found + active(context)).distinct().sortedBy { number(it) }
    }

    /**
     * Makes [slot] the active account and gives back the one it left, which
     * is let go, not closed: a worker that still holds it runs to its end.
     */
    @Synchronized
    fun switch(context: Context, slot: String): Samu? {
        val left = open
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(ACTIVE, slot).commit()
        open = null
        get(context)
        return left
    }

    /** A folder for one more account, not made yet. */
    fun fresh(context: Context): String = "samu-${(slots(context).maxOfOrNull { number(it) } ?: 0) + 1}"

    /** How [slot] stands, from its own database; the active one's from [get]. */
    fun status(context: Context, slot: String): de.samudo.app.mobile.Status? = runCatching {
        if (slot == active(context)) return@runCatching get(context).status()
        Samu.open(dir(context, slot)).use { it.status() }
    }.getOrNull()

    /** Signs [slot] out and throws its folder away (§4); never the active one. */
    fun remove(context: Context, slot: String) {
        check(slot != active(context)) { "the active account is not removed" }
        runCatching { Samu.open(dir(context, slot)).use { it.logout() } }
        context.filesDir.resolve(slot).deleteRecursively()
    }

    private fun dir(context: Context, slot: String) = context.filesDir.resolve(slot).path

    private fun number(slot: String) = slot.substringAfter("samu-", "1").toIntOrNull() ?: 1
}

/**
 * When the phone syncs (ADR 0028 §5): on opening, after every change, on
 * demand and every quarter of an hour with a network. All run [SyncWorker].
 */
object Sync {
    const val NOW = "sync"
    private const val PERIODIC = "sync-periodic"

    private val network =
        Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()

    /** A sync now; one that waits already is enough. */
    fun now(context: Context) {
        WorkManager.getInstance(context).enqueueUniqueWork(
            NOW,
            ExistingWorkPolicy.KEEP,
            OneTimeWorkRequestBuilder<SyncWorker>().setConstraints(network).build(),
        )
    }

    /**
     * After a change on the phone: after the running one, so it is pushed
     * too. While a „Rückgängig“ bar stands it waits; the bar syncs when it
     * is gone (ADR 0104 §2).
     */
    fun afterChange(context: Context) {
        if (Undo.holding) return
        WorkManager.getInstance(context).enqueueUniqueWork(
            NOW,
            ExistingWorkPolicy.APPEND_OR_REPLACE,
            OneTimeWorkRequestBuilder<SyncWorker>().setConstraints(network).build(),
        )
    }

    fun periodic(context: Context) {
        WorkManager.getInstance(context).enqueueUniquePeriodicWork(
            PERIODIC,
            ExistingPeriodicWorkPolicy.KEEP,
            PeriodicWorkRequestBuilder<SyncWorker>(15, TimeUnit.MINUTES)
                .setConstraints(network)
                .build(),
        )
    }
}

/**
 * Push the outbox, pull what changed. Without a server it tries again
 * later; signed out it stops, and the screen shows the sign-in.
 */
class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val samu = Store.get(applicationContext)
        if (!samu.status().signedIn) return@withContext Result.success()
        try {
            val report = samu.sync()
            Log.i("samu", "sync: $report")
            Widgets.refresh(applicationContext)
            Reminders.arm(applicationContext)
            Clocking.refresh(applicationContext)
            Result.success()
        } catch (e: MobileException.Offline) {
            Log.i("samu", "sync offline: ${e.reason}")
            Result.retry()
        } catch (e: MobileException) {
            Log.w("samu", "sync: $e")
            Result.success()
        }
    }
}
