// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.BlockerRow
import de.samudo.app.mobile.MobileException
import de.samudo.app.mobile.PlaceRow
import de.samudo.app.mobile.Samu
import de.samudo.app.mobile.TaskEdit
import de.samudo.app.mobile.TaskRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Inbox or an open project, with the folders as the tree (ADR 0040 §3). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MovePicker(current: String?, onPick: (String?) -> Unit, close: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var targets by remember { mutableStateOf<List<PlaceRow>>(emptyList()) }
    var query by remember { mutableStateOf("") }
    LaunchedEffect(Unit) {
        targets = withContext(Dispatchers.IO) { runCatching { Store.get(context).moveTargets() }.getOrDefault(emptyList()) }
    }
    ModalBottomSheet(onDismissRequest = close, sheetState = sheet, containerColor = t.pane) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
            Text(tr("Verschieben nach"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
            OutlinedTextField(
                query, { query = it }, singleLine = true,
                placeholder = { Text(tr("Projekt suchen")) },
                modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
            )
            val words = query.trim().lowercase()
            // While searching, only matching projects, without the tree.
            val shown = if (words.isEmpty()) targets else targets.filter { it.kind == "project" && it.title.lowercase().contains(words) }
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 480.dp)) {
                if (words.isEmpty()) {
                    item(key = "inbox") { Target("Inbox", 0, current == null, true) { onPick(null) } }
                }
                items(shown, key = { it.id }) { p ->
                    val project = p.kind == "project"
                    Target(
                        (if (project) "" else "▸ ") + p.title + (if (p.meta.isNotEmpty()) "  · ${p.meta}" else ""),
                        if (words.isEmpty()) p.depth.toInt() else 0,
                        p.id == current, project,
                    ) { onPick(p.id) }
                }
            }
        }
    }
}

/**
 * The open tasks `id` may wait for (ae5a3206), searched by title; samu-mobile
 * leaves out the task itself, its blockers and any that would close a circle.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BlockerPicker(id: String, onPick: (String) -> Unit, close: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var query by remember { mutableStateOf("") }
    var found by remember { mutableStateOf<List<BlockerRow>>(emptyList()) }
    LaunchedEffect(query) {
        found = withContext(Dispatchers.IO) {
            runCatching { Store.get(context).blockerTargets(id, query) }.getOrDefault(emptyList())
        }
    }
    ModalBottomSheet(onDismissRequest = close, sheetState = sheet, containerColor = t.pane) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
            Text(tr("Wartet auf"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
            OutlinedTextField(
                query, { query = it }, singleLine = true,
                placeholder = { Text(tr("Aufgabe suchen")) },
                modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
            )
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 480.dp)) {
                if (found.isEmpty()) item { Text(tr("Keine Aufgabe, die blockieren könnte."), color = t.muted, fontSize = 13.sp) }
                items(found, key = { it.taskId }) { b ->
                    Target(b.title + "  · " + (b.place ?: "Inbox"), 0, false, true) { onPick(b.taskId) }
                }
            }
        }
    }
}

@Composable
private fun Target(label: String, depth: Int, here: Boolean, pickable: Boolean, pick: () -> Unit) {
    val t = LocalTokens.current
    Text(
        label + if (here) "  ✓" else "",
        color = if (pickable) t.fg else t.muted,
        fontSize = 15.sp,
        fontWeight = if (here) FontWeight.SemiBold else FontWeight.Normal,
        modifier = Modifier.fillMaxWidth()
            .then(if (pickable && !here) Modifier.clickable(onClick = pick) else Modifier)
            .padding(start = (8 + 18 * depth).dp, top = 12.dp, bottom = 12.dp),
    )
}

/** A text that saves when the field loses focus or on "Fertig". */
@Composable
fun EditableText(
    value: String,
    placeholder: String,
    single: Boolean,
    big: Boolean = false,
    save: (String) -> Unit,
) {
    var text by remember(value) { mutableStateOf(value) }
    var focused by remember { mutableStateOf(false) }
    Column {
        OutlinedTextField(
            text, { text = it },
            singleLine = single,
            placeholder = { Text(placeholder) },
            textStyle = androidx.compose.ui.text.TextStyle(
                fontSize = if (big) 19.sp else 14.sp,
                fontWeight = if (big) FontWeight.SemiBold else FontWeight.Normal,
                color = LocalTokens.current.fg,
            ),
            modifier = Modifier.fillMaxWidth().onFocusChanged { state ->
                if (focused && !state.isFocused && text != value) save(text)
                focused = state.isFocused
            },
        )
        if (focused && text != value) {
            TextButton(onClick = { save(text) }) { Text(tr("Fertig")) }
        }
    }
}

/** Start, due and planned as whole days with the system's picker (ADR 0040 §2). */
@Composable
fun DateLine(label: String, value: String?, set: (String?) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(label, color = t.muted, fontSize = 13.sp, modifier = Modifier.width(96.dp))
        Text(
            value?.let { Words.day(it) } ?: "–",
            color = t.fg, fontSize = 14.sp,
            modifier = Modifier.weight(1f).clickable { pickDay(context, value) { set(it) } }.padding(vertical = 8.dp),
        )
        if (value != null) TextButton(onClick = { set(null) }) { Text(tr("weg")) }
    }
}

internal fun pickDay(context: android.content.Context, value: String?, then: (String) -> Unit) {
    val start = runCatching { java.time.LocalDate.parse(value?.take(10)) }.getOrNull() ?: java.time.LocalDate.now()
    android.app.DatePickerDialog(context, { _, year, month, day ->
        then(java.time.LocalDate.of(year, month + 1, day).toString())
    }, start.year, start.monthValue - 1, start.dayOfMonth).show()
}

private val WARNINGS = listOf(0L, 1L, 2L, 3L, 7L, 14L)

private fun warningWords(d: Long): String = when {
    d == 0L -> tr("am Tag")
    d % 7 == 0L -> if (d == 7L) tr("1 Woche") else tr("{n} Wochen", "n" to d / 7)
    d == 1L -> tr("1 Tag")
    else -> tr("{d} Tage", "d" to d)
}

/**
 * How long before „Fällig“ the task counts as due soon, its own (cbda7d51).
 * No chip chosen follows the account's default; a tap on the chosen one
 * goes back to it.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun DueWarningLine(id: String, edit: TaskEdit, act: (Samu.() -> Unit) -> Unit) {
    val t = LocalTokens.current
    val own = edit.dueWarningDays
    val default = edit.dueSoonDays?.let { tr("Vorgabe: {n}", "n" to warningWords(it)) } ?: tr("Vorgabe des Kontos")
    Text(tr("VORWARNUNG"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        WARNINGS.forEach { d ->
            val on = own == d
            FilterChip(
                selected = on,
                onClick = { act { setDueWarning(id, if (on) null else d) } },
                label = { Text(warningWords(d), fontSize = 13.sp, color = t.fg) },
            )
        }
        own?.takeIf { it !in WARNINGS }?.let { d ->
            FilterChip(selected = true, onClick = { act { setDueWarning(id, null) } }, label = { Text(warningWords(d), fontSize = 13.sp, color = t.fg) })
        }
        // Not whole days, as written elsewhere (`PT12H`).
        edit.dueWarning?.takeIf { own == null }?.let { raw ->
            FilterChip(selected = true, onClick = { act { setDueWarning(id, null) } }, label = { Text(raw, fontSize = 13.sp, color = t.fg) })
        }
    }
    if (edit.dueWarning == null) Text(default, color = t.muted, fontSize = 12.sp)
}

private val ESTIMATES = listOf(5L, 15L, 30L, 60L, 120L, 240L, 480L)

/** Estimate, tags and the workflow's state as chips (ADR 0040 §2). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun EditChips(id: String, edit: TaskEdit, act: (Samu.() -> Unit) -> Unit, show: (String) -> Boolean = { true }) {
    val t = LocalTokens.current
    if (show("state") && edit.states.isNotEmpty()) {
        Text(tr("ZUSTAND"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            edit.states.forEach { s ->
                FilterChip(
                    selected = s.on,
                    onClick = { act { setState(id, if (s.on) null else s.id) } },
                    label = { Text(s.path, fontSize = 13.sp, color = t.fg) },
                )
            }
        }
    }
    // ADR 0056 §6: none, A, B, C; a tap on the chosen one takes it away.
    if (show("priority")) {
        Text(tr("PRIORITÄT"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("A", "B", "C").forEach { p ->
                val on = edit.priority == p
                FilterChip(
                    selected = on,
                    onClick = { act { setPriority(id, if (on) null else p) } },
                    label = { Text(p, fontSize = 13.sp, color = t.fg) },
                )
            }
        }
    }
    if (show("estimate")) {
        Text(tr("SCHÄTZUNG"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            ESTIMATES.forEach { m ->
                val on = edit.estimateMinutes == m
                FilterChip(
                    selected = on,
                    onClick = { act { setEstimate(id, if (on) null else m) } },
                    label = { Text(Words.minutes(m), fontSize = 13.sp, color = t.fg) },
                )
            }
            edit.estimateMinutes?.takeIf { it !in ESTIMATES }?.let { m ->
                FilterChip(selected = true, onClick = { act { setEstimate(id, null) } }, label = { Text(Words.minutes(m), fontSize = 13.sp, color = t.fg) })
            }
        }
    }
    if (!show("tags")) return
    var naming by remember { mutableStateOf(false) }
    if (naming) NewTagDialog(close = { naming = false }) { tag -> naming = false; act { setTag(id, tag, true) } }
    Text(tr("TAGS"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        // The ones it carries first, so they are seen without scrolling.
        edit.tags.sortedBy { !it.on }.forEach { tag ->
            FilterChip(
                selected = tag.on,
                onClick = { act { setTag(id, tag.id, !tag.on) } },
                label = { Text("@" + tag.path, fontSize = 13.sp, color = t.fg) },
            )
        }
        NewTagChip { naming = true }
    }
}

/** The last chip of a tag picker: asks for a tag that is not there yet (92af4fff). */
@Composable
fun NewTagChip(onClick: () -> Unit) {
    val t = LocalTokens.current
    FilterChip(selected = false, onClick = onClick, label = { Text(tr("+ Neu"), fontSize = 13.sp, color = t.accent) })
}

/**
 * A name or a path like Ort/Garten: samu-core finds the tag by whole
 * title or makes what is missing (92af4fff); `made` gets its id. A
 * mistake (a workflow, two tags of that name) shows under the field.
 */
@Composable
fun NewTagDialog(close: () -> Unit, made: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val t = LocalTokens.current
    var path by remember { mutableStateOf("") }
    var problem by remember { mutableStateOf<String?>(null) }
    AlertDialog(
        onDismissRequest = close,
        title = { Text(tr("Neuer Tag")) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(path, { path = it; problem = null }, singleLine = true, label = { Text(tr("Name oder Ort/Name")) })
                problem?.let { Text(it, color = t.overdue, fontSize = 12.sp) }
            }
        },
        confirmButton = {
            TextButton(enabled = path.isNotBlank(), onClick = {
                scope.launch {
                    try {
                        made(withContext(Dispatchers.IO) { Store.get(context).createTagPath(path) })
                    } catch (e: Exception) {
                        problem = said(e)
                    }
                }
            }) { Text(tr("Setzen")) }
        },
        dismissButton = { TextButton(onClick = close) { Text(tr("Abbrechen")) } },
    )
}

/** The project line of the task page: tapping it moves. */
@Composable
fun ProjectLine(project: String?, open: () -> Unit) {
    val t = LocalTokens.current
    Row(
        Modifier.background(t.side, RoundedCornerShape(6.dp)).clickable(onClick = open).padding(horizontal = 10.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(project ?: "Inbox", color = t.fg, fontSize = 13.sp)
        Text(tr("  ▾ verschieben"), color = t.muted, fontSize = 12.sp)
    }
}

/** Shown in place of deleting when there is no server. */
@Composable
fun DeleteButton(id: String, changed: () -> Unit, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var asking by remember { mutableStateOf<String?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }
    val ask = asking
    if (ask == null) {
        OutlinedButton(onClick = {
            scope.launch {
                try {
                    val n = withContext(Dispatchers.IO) { Store.get(context).deleteTask(id, true) }
                    asking = if (n.deleted > 1uL) tr("Mit {n} Unteraufgaben löschen?", "n" to n.deleted - 1uL) else tr("Aufgabe löschen?")
                } catch (e: MobileException.Offline) {
                    problem = tr("Löschen geht nur mit Verbindung zum Server.")
                } catch (e: Exception) {
                    problem = e.message
                }
            }
        }) { Text(tr("Löschen …")) }
    } else {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(ask, color = t.overdue, fontSize = 14.sp)
            TextButton(onClick = {
                scope.launch {
                    try {
                        withContext(Dispatchers.IO) { Store.get(context).deleteTask(id, false) }
                        changed()
                        back()
                    } catch (e: Exception) {
                        problem = e.message
                    }
                }
            }) { Text(tr("Löschen"), color = t.overdue) }
            TextButton(onClick = { asking = null }) { Text(tr("Nein")) }
        }
    }
    problem?.let { Text(it, color = t.overdue, fontSize = 13.sp) }
}
