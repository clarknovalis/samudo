// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.Chip
import de.samudo.app.mobile.EditChip
import de.samudo.app.mobile.EditPreview
import de.samudo.app.mobile.Offer
import de.samudo.app.mobile.Samu
import de.samudo.app.mobile.TaskRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * What a task's menu hands up to the screen: the task a subtask is being
 * captured under (ADR 0062 §2). Held here like [Picks], so no list has to
 * pass it through.
 */
object TaskMenus {
    val under = mutableStateOf<TaskRow?>(null)

    /** The task in the edit dialog (ADR 0078 §6). */
    val editing = mutableStateOf<TaskRow?>(null)
}

/**
 * The menu of a task on a long press (ADR 0102 §7, ADR 0062 §5): a sheet
 * from below with the title as its head. Each entry goes the way its
 * button on the task page goes; the menu is a way in, not new behaviour.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TaskMenu(row: TaskRow, act: (Samu.() -> Unit) -> Unit, flag: () -> Unit, close: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var moving by remember { mutableStateOf(false) }
    val open = !row.done && !row.dropped
    if (moving) {
        MovePicker(row.projectId, onPick = { target ->
            moving = false
            act { moveTask(row.id, target) }
            close()
        }, close = { moving = false })
        return
    }
    ModalBottomSheet(onDismissRequest = close, sheetState = sheet, containerColor = t.pane) {
        Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
            Text(
                row.title, color = t.fg, fontSize = 16.sp, fontWeight = FontWeight.SemiBold,
                maxLines = 2, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(20.dp, 0.dp, 20.dp, 8.dp),
            )
            val item: @Composable (String, () -> Unit) -> Unit = { label, onClick ->
                Text(
                    label, color = t.fg, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 20.dp, vertical = 14.dp),
                )
            }
            item(tr("Bearbeiten")) { close(); TaskMenus.editing.value = row }
            if (open) item(tr("Unteraufgabe anlegen")) { close(); TaskMenus.under.value = row }
            item(if (row.flagged) tr("Markierung weg") else tr("⚑ Markieren")) { flag(); close() }
            if (open) {
                item(tr("Zurückstellen bis …")) {
                    pickDay(context, row.defer) { day -> act { setDate(row.id, "defer", day) } }
                    close()
                }
            }
            item(tr("Verschieben …")) { moving = true }
            HorizontalDivider(color = t.rowline)
            item(tr("Auswählen")) { close(); Picks.start(row) }
        }
    }
}

/** A chip of the edit dialog in the words of the capture preview. */
private fun chipText(c: EditChip): String = when (c.kind) {
    "habit" -> "Habit"
    "reminder" -> tr("Erinnerung")
    "clock" -> tr("Zeit gebucht")
    "note" -> tr("Notiz +")
    else -> Words.chip(Chip(c.kind, c.value, emptyList()))
}

/**
 * „Bearbeiten“ (ADR 0078 §1, §6): the title in its own field, what is set
 * as chips (✕ marks one for removal, ↶ takes that back), and a line in the
 * quick-capture syntax whose preview shows what it adds. „Sichern“ writes
 * it all at once, offline; samu-mobile applies the line as the server
 * would.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun EditSheet(row: TaskRow, close: (Boolean) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var title by remember(row.id) { mutableStateOf(row.title) }
    var field by remember(row.id) { mutableStateOf(TextFieldValue("")) }
    var chips by remember(row.id) { mutableStateOf<List<EditChip>>(emptyList()) }
    var removed by remember(row.id) { mutableStateOf(emptySet<String>()) }
    var preview by remember { mutableStateOf<EditPreview?>(null) }
    var offer by remember { mutableStateOf<Offer?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val focus = remember { FocusRequester() }
    val text = field.text
    LaunchedEffect(row.id) {
        chips = withContext(Dispatchers.IO) { runCatching { Store.get(context).editChips(row.id) }.getOrDefault(emptyList()) }
        focus.requestFocus()
    }
    LaunchedEffect(text) {
        delay(120)
        preview = if (text.isBlank()) null else withContext(Dispatchers.IO) {
            runCatching { Store.get(context).editPreview(row.id, text) }.getOrNull()
        }
    }
    LaunchedEffect(text, field.selection) {
        val cursor = field.selection.start.toUInt()
        offer = withContext(Dispatchers.IO) { runCatching { Store.get(context).captureOffer(text, cursor) }.getOrNull() }
    }
    val save: () -> Unit = {
        scope.launch {
            try {
                withContext(Dispatchers.IO) { Store.get(context).editSave(row.id, title, removed.toList(), text) }
                close(true)
            } catch (e: Exception) {
                error = said(e)
            }
        }
    }
    val pill: @Composable (String, androidx.compose.ui.graphics.Color, androidx.compose.ui.graphics.Color, Boolean, (() -> Unit)?) -> Unit =
        { label, bg, fg, struck, onClick ->
            Text(
                label, color = fg, fontSize = 12.sp,
                textDecoration = if (struck) TextDecoration.LineThrough else null,
                modifier = Modifier.background(bg, RoundedCornerShape(999.dp))
                    .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
                    .padding(10.dp, 3.dp),
            )
        }
    ModalBottomSheet(onDismissRequest = { close(false) }, sheetState = sheet, containerColor = t.pane) {
        Column(Modifier.fillMaxWidth().imePadding().padding(16.dp, 0.dp, 16.dp, 24.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(tr("Bearbeiten"), color = t.muted, fontSize = 13.sp)
            OutlinedTextField(title, { title = it }, label = { Text(tr("Titel")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            if (chips.isNotEmpty()) {
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    chips.forEach { c ->
                        val off = c.key in removed
                        pill(chipText(c) + if (off) "  ↶" else "  ✕", t.rowline, if (off) t.muted else t.fg, off) {
                            removed = if (off) removed - c.key else removed + c.key
                        }
                    }
                }
            }
            OutlinedTextField(
                field, { field = it },
                placeholder = { Text("@tag d:fr !A ~30m #projekt // Notiz") },
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { save() }),
            )
            offer?.let { o ->
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    o.items.forEach { name ->
                        Text(o.sign + name, color = t.accent, fontSize = 13.sp,
                            modifier = Modifier
                                .border(1.dp, t.accent, RoundedCornerShape(999.dp))
                                .clickable { field = accepted(field, o, name); offer = null }
                                .padding(12.dp, 6.dp))
                    }
                }
            }
            preview?.let { p ->
                // What the line adds, highlighted.
                if (p.added.isNotEmpty()) {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        p.added.forEach { c -> pill("+ " + chipText(c), t.okBg, t.ok, false, null) }
                    }
                }
                p.warnings.forEach { w ->
                    val said = if (w.code == "free_text") tr("„{text}“: kein Kürzel; der Titel steht oben", "text" to w.text) else Words.warning(w)
                    Text("• $said", color = t.wait, fontSize = 13.sp)
                }
            }
            error?.let { Text(it, color = t.overdue) }
            Button(onClick = save, enabled = title.isNotBlank(), modifier = Modifier.align(Alignment.End)) { Text(tr("Sichern")) }
        }
    }
}
