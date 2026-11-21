// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.material3.Checkbox
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.ChecklistRow
import de.samudo.app.mobile.Samu
import de.samudo.app.mobile.TaskRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The checklist of a task (ADR 0141): tick, edit, move with ↑/↓, remove,
 * and add; a pasted list becomes one item per line. Hidden on a closed
 * task that has none.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ChecklistPart(id: String, row: TaskRow, tick: Int, changed: () -> Unit, act: (Samu.() -> Unit) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val t = LocalTokens.current
    var items by remember { mutableStateOf<List<ChecklistRow>>(emptyList()) }
    var adding by remember { mutableStateOf("") }
    // The adder stays visible above the keyboard: on focus, and again when
    // an added item pushes it down.
    val adderView = remember { BringIntoViewRequester() }
    var adderFocused by remember { mutableStateOf(false) }
    LaunchedEffect(items.size, adderFocused) { if (adderFocused) adderView.bringIntoView() }
    LaunchedEffect(tick, id) {
        items = withContext(Dispatchers.IO) { runCatching { Store.get(context).checklist(id) }.getOrDefault(emptyList()) }
    }
    val closed = row.done || row.dropped
    if (items.isEmpty() && closed) return
    Column {
        val done = items.count { it.checked }
        Text(
            tr("CHECKLISTE") + if (items.isNotEmpty()) " $done/${items.size}" else "",
            color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold,
        )
        items.forEachIndexed { index, item ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(item.checked, { on -> act { runCatching { checklistCheck(item.id, on) } } })
                Box(Modifier.weight(1f)) {
                    EditableText(item.text, tr("Checkpunkt"), single = true) { text ->
                        if (text.isNotBlank()) act { runCatching { checklistEdit(item.id, text) } }
                    }
                }
                if (index > 0) TextButton(onClick = { act { runCatching { checklistMove(item.id, (index - 1).toUInt()) } } }) { Text("↑") }
                if (index < items.size - 1) TextButton(onClick = { act { runCatching { checklistMove(item.id, (index + 1).toUInt()) } } }) { Text("↓") }
                TextButton(onClick = { act { runCatching { checklistRemove(item.id) } } }) { Text("✕") }
            }
            if (!closed) {
                TextButton(onClick = { act { runCatching { checklistToSubtask(item.id) } } }) {
                    Text(tr("In Unteraufgabe umwandeln"), fontSize = 12.sp)
                }
            }
        }
        if (!closed && (items.isNotEmpty() || row.group)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (items.isNotEmpty()) {
                    TextButton(onClick = { act { runCatching { checklistAllToSubtasks(id) } } }) {
                        Text(tr("Alle in Unteraufgaben"), fontSize = 12.sp)
                    }
                }
                if (row.group) {
                    TextButton(onClick = {
                        scope.launch {
                            try {
                                val kept = withContext(Dispatchers.IO) { Store.get(context).checklistAllFromSubtasks(id) }
                                changed()
                                if (kept.isNotEmpty()) {
                                    android.widget.Toast.makeText(context, kept.joinToString("\n"), android.widget.Toast.LENGTH_LONG).show()
                                }
                            } catch (e: Exception) {
                                android.widget.Toast.makeText(context, said(e), android.widget.Toast.LENGTH_LONG).show()
                            }
                        }
                    }) { Text(tr("Alle Unteraufgaben in Punkte"), fontSize = 12.sp) }
                }
            }
        }
        if (!closed) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    adding, { adding = it },
                    placeholder = { Text(tr("Punkt hinzufügen …")) },
                    modifier = Modifier.weight(1f).bringIntoViewRequester(adderView)
                        .onFocusChanged { adderFocused = it.isFocused },
                )
                TextButton(
                    enabled = adding.isNotBlank(),
                    onClick = {
                        val text = adding
                        adding = ""
                        act { runCatching { checklistAdd(id, text) } }
                    },
                ) { Text("+") }
            }
        }
    }
}

/**
 * Turns a subtask into an item of its parent's checklist (ADR 0141). The
 * server-side rule refuses what an item cannot keep and says what; the
 * task page closes after it, since the task is gone.
 */
@Composable
fun ToItemButton(id: String, changed: () -> Unit, back: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    OutlinedButton(onClick = {
        scope.launch {
            try {
                withContext(Dispatchers.IO) { Store.get(context).checklistFromSubtask(id) }
                changed()
                back()
            } catch (e: Exception) {
                android.widget.Toast.makeText(context, said(e), android.widget.Toast.LENGTH_LONG).show()
            }
        }
    }) { Text(tr("In Checkpunkt umwandeln")) }
}
