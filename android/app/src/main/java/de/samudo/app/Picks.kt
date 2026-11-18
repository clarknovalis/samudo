// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.BulkEdit
import de.samudo.app.mobile.MobileException
import de.samudo.app.mobile.Samu
import de.samudo.app.mobile.TaskRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The picked tasks (ADR 0052 §6): „Auswählen“ in a task's menu or in the
 * header's ⋮ starts picking (ADR 0102 §7, ADR 0062 §5), a tap or a long
 * press then takes a task in or out; ✕ or Back ends it, even with none
 * picked. Held here, beside the lists, so the bar at the top and every list
 * see the same picks.
 */
object Picks {
    val rows = mutableStateOf<List<TaskRow>>(emptyList())
    private val on = mutableStateOf(false)

    val active: Boolean get() = on.value || rows.value.isNotEmpty()

    /** Starts picking, with `row` picked when there is one. */
    fun start(row: TaskRow? = null) {
        on.value = true
        if (row != null && !has(row.id)) rows.value = rows.value + row
    }

    fun has(id: String) = rows.value.any { it.id == id }

    fun toggle(row: TaskRow) {
        rows.value = if (has(row.id)) rows.value.filterNot { it.id == row.id } else rows.value + row
    }

    fun clear() {
        on.value = false
        if (rows.value.isNotEmpty()) rows.value = emptyList()
    }

    /** The rows a list now shows replace the picked ones; picks it no longer shows fall away. */
    fun refresh(shown: List<TaskRow>) {
        if (rows.value.isEmpty()) return
        val byId = shown.associateBy { it.id }
        val fresh = rows.value.mapNotNull { byId[it.id] }
        if (fresh != rows.value) rows.value = fresh
    }

    val ids: List<String> get() = rows.value.map { it.id }
}

/** The bar in place of the title while picking: ✕, the count, the main actions and ⋯. */
@Composable
fun PickBar(changed: () -> Unit, edit: (String) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var more by remember { mutableStateOf(false) }
    var moving by remember { mutableStateOf(false) }
    val rows = Picks.rows.value
    val act: (Samu.(List<TaskRow>) -> Unit) -> Unit = { action ->
        val now = Picks.rows.value
        scope.launch {
            withContext(Dispatchers.IO) { Store.get(context).action(now) }
            changed()
        }
    }
    // Completing and flagging offer „Rückgängig“ (ADR 0104 §1).
    val undoable: (String, Samu.() -> List<Long>, Samu.() -> Unit) -> Unit = { text, action, back ->
        scope.launch {
            runCatching { Undo.run(context, text, action, back) }
            changed()
        }
    }
    Row(Modifier.fillMaxWidth().padding(4.dp, 12.dp, 4.dp, 4.dp), verticalAlignment = Alignment.CenterVertically) {
        TextButton(onClick = { Picks.clear() }) { Text("✕", fontSize = 18.sp, color = t.muted) }
        Text(tr("{rows} ausgewählt", "rows" to rows.size), fontSize = 18.sp, fontWeight = FontWeight.SemiBold, color = t.fg, modifier = Modifier.weight(1f))
        if (rows.any { !it.done && !it.dropped }) {
            TextButton(onClick = {
                val open = Picks.rows.value.filter { !it.done && !it.dropped }
                // One question for the whole pick (ADR 0128 §5).
                scope.launch {
                    runCatching { Closing.run(context, open.map { it.id }, dropped = false, title = open.singleOrNull()?.title) }
                    changed()
                }
            }) { Text("✓") }
        }
        if (rows.isNotEmpty()) {
            TextButton(onClick = {
                val on = rows.any { !it.flagged }
                val those = Picks.rows.value.filter { it.flagged != on }
                val text = if (on) Undo.count(those.size, tr("Markiert"), tr("markiert")) else Undo.count(those.size, tr("Markierung weg"), tr("ohne Markierung"))
                undoable(text, { those.flatMap { setFlag(it.id, on) } }, { those.forEach { setFlag(it.id, !on) } })
            }) { Text("⚑", color = t.flag, fontSize = 18.sp) }
            TextButton(onClick = { moving = true }) { Text(tr("Verschieben")) }
            TextButton(onClick = { more = true }) { Text("⋯", fontSize = 20.sp) }
        }
    }
    if (moving) {
        // No target counts as "here": the picks may come from anywhere.
        MovePicker("", onPick = { target ->
            moving = false
            act { list -> moveTasks(list.map { it.id }, target) }
        }, close = { moving = false })
    }
    if (more) PickMenu(changed, edit) { more = false }
}

/** ⋯: the rest of ADR 0052 §2; with one task also what its old long-press menu had (ADR 0040). */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
private fun PickMenu(changed: () -> Unit, edit: (String) -> Unit, close: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val rows = Picks.rows.value
    val ids = Picks.ids
    var info by remember { mutableStateOf<BulkEdit?>(null) }
    var page by remember { mutableStateOf("main") }
    var asking by remember { mutableStateOf(false) }
    var naming by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(ids) {
        info = withContext(Dispatchers.IO) { runCatching { Store.get(context).bulkEdit(ids) }.getOrNull() }
    }
    val act: (Samu.() -> Unit) -> Unit = { action ->
        scope.launch {
            try {
                withContext(Dispatchers.IO) { Store.get(context).action() }
                changed()
                close()
            } catch (e: Exception) {
                problem = said(e)
            }
        }
    }
    ModalBottomSheet(onDismissRequest = close, sheetState = sheet, containerColor = t.pane) {
        Column(Modifier.fillMaxWidth().heightIn(max = 560.dp).verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
            Text(
                if (rows.size == 1) rows[0].title else tr("{rows} Aufgaben", "rows" to rows.size),
                color = t.fg, fontSize = 16.sp, fontWeight = FontWeight.SemiBold,
                modifier = Modifier.padding(20.dp, 0.dp, 20.dp, 8.dp),
            )
            val item: @Composable (String, () -> Unit) -> Unit = { label, onClick ->
                Text(
                    label, color = t.fg, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 20.dp, vertical = 14.dp),
                )
            }
            when (page) {
                "tags" -> {
                    Text(tr("Tags: antippen setzt bei allen, nochmals nimmt ab"), color = t.muted, fontSize = 12.sp, modifier = Modifier.padding(horizontal = 20.dp))
                    FlowRow(Modifier.padding(16.dp, 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        info?.tags?.forEach { tag ->
                            FilterChip(
                                selected = tag.on,
                                onClick = { act { ids.forEach { setTag(it, tag.id, !tag.on) } } },
                                label = { Text(tag.path, fontSize = 13.sp, color = t.fg) },
                            )
                        }
                        NewTagChip { naming = true }
                    }
                    if (naming) {
                        NewTagDialog(close = { naming = false }) { tag -> naming = false; act { ids.forEach { setTag(it, tag, true) } } }
                    }
                }
                "state" -> {
                    item(tr("kein Zustand")) { act { setStatesTitled(ids, null) } }
                    info?.states?.forEach { title ->
                        item(title) {
                            scope.launch {
                                val missed = withContext(Dispatchers.IO) { Store.get(context).setStatesTitled(ids, title) }
                                changed()
                                if (missed > 0u) problem = tr("{missed} ohne „{title}“ im Ablauf, unverändert.", "missed" to missed, "title" to title) else close()
                            }
                        }
                    }
                }
                "dates" -> listOf("due" to tr("Fällig"), "planned" to tr("Geplant"), "defer" to tr("Start")).forEach { (field, label) ->
                    Row(Modifier.padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(label, color = t.muted, fontSize = 14.sp, modifier = Modifier.padding(start = 8.dp).weight(1f))
                        TextButton(onClick = { act { ids.forEach { planOn(this, it, field, 0) } } }) { Text(tr("Heute")) }
                        TextButton(onClick = { act { ids.forEach { planOn(this, it, field, 1) } } }) { Text(tr("Morgen")) }
                        TextButton(onClick = { pickDay(context, null) { day -> act { ids.forEach { setDate(it, field, day) } } } }) { Text(tr("Am …")) }
                        TextButton(onClick = { act { ids.forEach { setDate(it, field, null) } } }) { Text(tr("Leeren")) }
                    }
                }
                else -> {
                    if (rows.size == 1) {
                        val row = rows[0]
                        item(tr("Bearbeiten")) { close(); Picks.clear(); edit(row.id) }
                        if (!row.done && !row.dropped) {
                            item(tr("◷ Uhr starten / stoppen")) {
                                act { if (runningClock()?.ownerId == row.id) clockOut() else clockIn(row.id) }
                            }
                        }
                    }
                    if (rows.any { it.done || it.dropped }) item(tr("Wieder öffnen")) { act { rows.filter { it.done || it.dropped }.forEach { reopen(it.id) } } }
                    if (rows.any { !it.dropped }) {
                        item(tr("Verwerfen")) {
                            // With „Rückgängig“ (ADR 0104 §1).
                            val those = rows.filter { !it.dropped }
                            val open = those.filter { !it.done }
                            val done = those.filter { it.done }
                            scope.launch {
                                try {
                                    if (open.isEmpty()) {
                                        Undo.run(context, Undo.count(done.size, tr("Verworfen"), tr("verworfen")), { done.flatMap { dropTask(it.id) } }, { done.forEach { reopen(it.id) } })
                                    } else {
                                        // Completed ones are dropped as they are; the open ones
                                        // ask first with open tasks below (ADR 0128 §5).
                                        withContext(Dispatchers.IO) { Store.get(context).run { done.forEach { dropTask(it.id) } } }
                                        Closing.run(context, open.map { it.id }, dropped = true, title = open.singleOrNull()?.title)
                                    }
                                    changed()
                                    close()
                                } catch (e: Exception) {
                                    problem = said(e)
                                }
                            }
                        }
                    }
                    item(tr("Heute planen")) { act { ids.forEach { planIn(it, 0) } } }
                    item(tr("Morgen planen")) { act { ids.forEach { planIn(it, 1) } } }
                    item(tr("Termine …")) { page = "dates" }
                    item(tr("Tags …")) { page = "tags" }
                    if (info?.states?.isNotEmpty() == true) item(tr("Zustand …")) { page = "state" }
                    if (!asking) {
                        item(tr("Löschen …")) { asking = true }
                    } else {
                        val n = info?.deleted ?: rows.size.toULong()
                        Row(Modifier.padding(horizontal = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text(if (n == 1uL) tr("1 Aufgabe löschen, Unteraufgaben mitgezählt?") else tr("{n} Aufgaben löschen, Unteraufgaben mitgezählt?", "n" to n), color = t.overdue, fontSize = 15.sp, modifier = Modifier.weight(1f))
                            TextButton(onClick = {
                                scope.launch {
                                    try {
                                        withContext(Dispatchers.IO) { Store.get(context).deleteTasks(ids, false) }
                                        Picks.clear()
                                        changed()
                                        close()
                                    } catch (e: MobileException.Offline) {
                                        problem = tr("Löschen geht nur mit Verbindung zum Server.")
                                    } catch (e: Exception) {
                                        problem = said(e)
                                    }
                                }
                            }) { Text(tr("Löschen"), color = t.overdue) }
                            TextButton(onClick = { asking = false }) { Text(tr("Nein")) }
                        }
                    }
                }
            }
            if (page != "main") TextButton(onClick = { page = "main" }, modifier = Modifier.padding(horizontal = 12.dp)) { Text(tr("‹ Zurück")) }
            problem?.let { Text(it, color = t.overdue, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp)) }
        }
    }
}

/** A date `days` from today in the account's zone, like `planIn` but for any of the three fields. */
private fun planOn(samu: Samu, id: String, field: String, days: Long) {
    if (field == "planned") {
        samu.planIn(id, days)
        return
    }
    val day = java.time.LocalDate.now(java.time.ZoneId.of(samu.status().zone)).plusDays(days).toString()
    samu.setDate(id, field, day)
}


