// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.Closed
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Completing and dropping tasks (ADR 0128 §5): every way in the app goes
 * through [run]. With open tasks below that are not picked as well, it
 * asks first in [CloseSheet]: close them along, lift them out one level,
 * or leave it. What it wrote, „Rückgängig“ takes back (ADR 0104).
 */
object Closing {
    /** A question standing: [after] runs once it is answered by closing. */
    class Ask(
        val ids: List<String>,
        val dropped: Boolean,
        val title: String?,
        val asking: Int,
        val open: Long,
        val after: () -> Unit,
    )

    val ask = mutableStateOf<Ask?>(null)

    private fun field(dropped: Boolean) = if (dropped) "dropped" else "completed"

    /**
     * Closes [ids], or asks first; [title] names a single task in the
     * question. [after] runs once something was closed, not when it asks.
     */
    suspend fun run(context: Context, ids: List<String>, dropped: Boolean, title: String? = null, after: () -> Unit = {}) {
        if (ids.isEmpty()) return
        val done = withContext(Dispatchers.IO) { Store.get(context).closeTasks(ids, field(dropped), "ask") }
        if (done.asking > 0u) {
            ask.value = Ask(ids, dropped, title.takeIf { ids.size == 1 }, done.asking.toInt(), done.open.toLong(), after)
            return
        }
        offer(done, ids.size, dropped)
        after()
    }

    /** The answer to [a]: `close` or `lift`. */
    suspend fun answer(context: Context, a: Ask, children: String) {
        val done = withContext(Dispatchers.IO) { Store.get(context).closeTasks(a.ids, field(a.dropped), children) }
        offer(done, a.ids.size, a.dropped)
        a.after()
    }

    private fun offer(done: Closed, n: Int, dropped: Boolean) {
        if (done.numbers.isEmpty()) return
        val text = if (dropped) Undo.count(n, tr("Verworfen"), tr("verworfen")) else Undo.count(n, tr("Erledigt"), tr("erledigt"))
        val back = done.back
        Undo.offer.value = Undo.Offer(text, done.numbers) { putBack(back) }
    }
}

/** The question of [Closing], from below; [changed] draws after an answer. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CloseSheet(a: Closing.Ask, changed: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var problem by remember { mutableStateOf<String?>(null) }
    val close = { Closing.ask.value = null }
    val answer: (String) -> Unit = { children ->
        scope.launch {
            try {
                Closing.answer(context, a, children)
                close()
                changed()
            } catch (e: Exception) {
                problem = said(e)
            }
        }
    }
    val question = if (a.title != null) {
        tr("„{title}“ hat {n} offene Unteraufgaben.", "title" to a.title, "n" to a.open)
    } else {
        tr("{k} der {n} Aufgaben haben offene Unteraufgaben.", "k" to a.asking, "n" to a.ids.size)
    }
    ModalBottomSheet(onDismissRequest = close, sheetState = sheet, containerColor = t.pane) {
        Column(
            Modifier.fillMaxWidth().padding(20.dp, 0.dp, 20.dp, 24.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(question, color = t.fg, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
            Text(
                if (a.dropped) {
                    tr("Mit verwerfen: alle offenen darunter auch. Herausholen: sie rücken eine Ebene höher.")
                } else {
                    tr("Mit erledigen: alle offenen darunter auch. Herausholen: sie rücken eine Ebene höher.")
                },
                color = t.muted, fontSize = 13.sp,
            )
            // „Mit erledigen“ first and filled (ADR 0128, F1).
            Button(onClick = { answer("close") }, modifier = Modifier.fillMaxWidth()) {
                Text(if (a.dropped) tr("Mit verwerfen") else tr("Mit erledigen"))
            }
            OutlinedButton(onClick = { answer("lift") }, modifier = Modifier.fillMaxWidth()) { Text(tr("Herausholen")) }
            TextButton(onClick = close, modifier = Modifier.fillMaxWidth()) { Text(tr("Abbrechen")) }
            problem?.let { Text(it, color = t.overdue, fontSize = 13.sp) }
        }
    }
}
