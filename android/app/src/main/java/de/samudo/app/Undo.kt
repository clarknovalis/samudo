// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.content.Context
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.platform.LocalContext
import de.samudo.app.mobile.Samu
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * „Rückgängig“ after completing, flagging or dropping from a list (ADR
 * 0104): while the bar stands, nothing syncs; taking it back withdraws the
 * changes from the outbox, or, once sent, writes [Offer.back].
 */
object Undo {
    class Offer(val text: String, val numbers: List<Long>, val back: Samu.() -> Unit)

    val host = SnackbarHostState()
    val offer = mutableStateOf<Offer?>(null)

    /** A bar stands: `Sync.afterChange` waits for it (ADR 0104 §2). */
    val holding: Boolean get() = offer.value != null

    /**
     * Does [action] on the phone and offers to take it back; the caller
     * draws again after and says what failed. A newer offer replaces the
     * one before.
     */
    suspend fun run(context: Context, text: String, action: Samu.() -> List<Long>, back: Samu.() -> Unit) {
        val numbers = withContext(Dispatchers.IO) { Store.get(context).action() }
        if (numbers.isNotEmpty()) offer.value = Offer(text, numbers, back)
    }

    /** „Erledigt“ for one, „3 erledigt“ for more. */
    fun count(n: Int, one: String, many: String): String = if (n == 1) one else "$n $many"
}

/** The bar itself, in the scaffold; [changed] draws and syncs once it is gone. */
@Composable
fun UndoHost(changed: () -> Unit) {
    val context = LocalContext.current
    val offer = Undo.offer.value
    LaunchedEffect(offer) {
        offer ?: return@LaunchedEffect
        try {
            val result = Undo.host.showSnackbar(offer.text, tr("Rückgängig"), duration = SnackbarDuration.Short)
            if (result == SnackbarResult.ActionPerformed) {
                withContext(Dispatchers.IO) {
                    runCatching {
                        val samu = Store.get(context)
                        if (!samu.withdraw(offer.numbers)) samu.(offer.back)()
                    }
                }
            }
        } finally {
            // Replaced: the newer one syncs when it is gone.
            if (Undo.offer.value === offer) {
                Undo.offer.value = null
                changed()
            }
        }
    }
    SnackbarHost(Undo.host)
}
