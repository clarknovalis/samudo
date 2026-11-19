// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.Due
import de.samudo.app.mobile.RowFields
import de.samudo.app.mobile.TaskRow
import java.time.Instant
import java.time.ZoneId
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * What a row shows besides its title (ADR 0130, 0131): line 1 is a list of
 * fields with the title as a fixed piece in it, line 2 another, and what
 * neither names is off; the short form is a switch. One layout for every
 * list on this phone, kept in its own cache by samu-mobile.
 */
object RowLayout {
    /** The fields in order, with their words in the sheet; the keys are samu-mobile's. */
    fun fields() = listOf(
        "due" to tr("Fällig"), "state" to tr("Zustand"), "tags" to tr("Tags"), "place" to tr("Ort"),
        "reason" to tr("Grund"), "defer" to tr("Start"), "planned" to tr("Geplant"),
        "estimate" to tr("Schätzung"), "repeat" to tr("Wiederholung"), "note" to tr("Notiz"),
        "created" to tr("erstellt"), "modified" to tr("geändert"),
    )

    const val TITLE = "title"

    /** Until the cache answers: the phone's default (ADR 0131 §6). */
    private val START = RowFields(listOf("state", TITLE, "due"), listOf("tags"), true)

    val layout = mutableStateOf(START)
    var open by mutableStateOf(false)

    /** 1, 2, or 0 when the field is off. */
    fun lineOf(l: RowFields, key: String): Int = when (key) {
        in l.line1 -> 1
        in l.line2 -> 2
        else -> 0
    }

    /** The keys of a row of the sheet; off in the order of [fields]. */
    fun keysOf(l: RowFields, line: Int): List<String> = when (line) {
        1 -> l.line1
        2 -> l.line2
        else -> fields().map { it.first }.filter { lineOf(l, it) == 0 }
    }

    /** `key` into `line` (1, 2, or 0 for off) at `index`; the title stays in line 1. */
    fun moved(l: RowFields, key: String, line: Int, index: Int = Int.MAX_VALUE): RowFields {
        if (key == TITLE && line != 1) return l
        val one = l.line1.filter { it != key }.toMutableList()
        val two = l.line2.filter { it != key }.toMutableList()
        when (line) {
            1 -> one.add(index.coerceIn(0, one.size), key)
            2 -> two.add(index.coerceIn(0, two.size), key)
        }
        return l.copy(line1 = one, line2 = two)
    }

    /** On the main thread; the cache is written off it and gives the settled layout back. */
    suspend fun set(samu: de.samudo.app.mobile.Samu, next: RowFields) {
        layout.value = next
        withContext(Dispatchers.IO) { runCatching { samu.setRowFields(next) } }.getOrNull()?.let { layout.value = it }
    }

    suspend fun load(samu: de.samudo.app.mobile.Samu) {
        withContext(Dispatchers.IO) { runCatching { samu.rowFields() } }.getOrNull()?.let { layout.value = it }
    }
}

/** One field of a row: what it says, its long form for a screen reader, and its colour. */
data class RowPart(val text: String, val label: String, val due: Due? = null)

private fun localDay(ms: Long): String = Instant.ofEpochMilli(ms).atZone(ZoneId.systemDefault()).toLocalDate().toString()

/** "PT1H30M" → "1h30", "PT45M" → "45m". */
private fun estimateShort(text: String): String {
    val m = Regex("""^PT(?:(\d+)H)?(?:(\d+)M)?$""").find(text) ?: return text
    val (h, min) = m.destructured
    return when {
        h.isNotEmpty() && min.isNotEmpty() -> "${h}h$min"
        h.isNotEmpty() -> "${h}h"
        else -> "${min}m"
    }
}

private val UNIT = mapOf("DAILY" to "d", "WEEKLY" to "w", "MONTHLY" to "M", "YEARLY" to "y")

/** Where a field stands in a row: before or after the title in line 1, or in line 2. */
enum class Slot { BEFORE, AFTER, LINE2 }

/**
 * The fields of one part of a row, in order, without the empty ones.
 * `place` is what the list says of where the task lives; none where its
 * heading says it already.
 */
fun rowParts(row: TaskRow, slot: Slot, place: String?): List<RowPart> {
    val layout = RowLayout.layout.value
    val short = layout.short
    val open = !row.done && !row.dropped
    fun make(long: String, brief: String = long, due: Due? = null) = RowPart(if (short) brief else long, long, due)
    val at = layout.line1.indexOf(RowLayout.TITLE)
    val keys = when (slot) {
        Slot.BEFORE -> layout.line1.take(at.coerceAtLeast(0))
        Slot.AFTER -> layout.line1.drop(at + 1)
        Slot.LINE2 -> layout.line2
    }
    return keys.mapNotNull { key ->
        when (key) {
            "due" -> if (open) Words.due(row)?.let { make(it, Words.day(row.due), row.dueState) } else null
            "state" -> row.state?.substringAfterLast('/')?.let { make(it) }
            "tags" -> row.tags.takeIf { it.isNotEmpty() }?.let { tags ->
                make(tags.joinToString(" ") { "@$it" }, tags.joinToString(" ") { "@" + it.substringAfterLast('/') })
            }
            "place" -> place?.let { make(it, it.substringBefore(" · ")) }
            "reason" -> if (open && !row.available) row.reason?.let { val r = Words.reason(it); make(r, "⊘ $r") } else null
            "defer" -> row.defer?.let { make(tr("Start {n}", "n" to Words.day(it)), "▶ " + Words.day(it)) }
            "planned" -> row.planned?.let { make(tr("Geplant {n}", "n" to Words.day(it)), "◷ " + Words.day(it)) }
            "estimate" -> row.estimate?.let { make("~" + Words.estimate(it), "~" + estimateShort(it)) }
            "repeat" -> row.repeat?.let { r ->
                make("↻ " + repeatText(r), "↻" + (UNIT[r.freq]?.let { "${r.interval}$it" } ?: ""))
            }
            "note" -> row.noteLine?.let { make(it, "¶ $it") }
            "created" -> make(tr("erstellt {day}", "day" to Words.day(localDay(row.createdMs))), "+" + Words.day(localDay(row.createdMs)))
            "modified" -> make(tr("geändert {day}", "day" to Words.day(localDay(row.modifiedMs))), "Δ" + Words.day(localDay(row.modifiedMs)))
            else -> null
        }
    }
}

/** The parts as one line: ` · ` between them, the due date in its colour. */
fun rowText(parts: List<RowPart>, muted: Color, overdue: Color, soon: Color): AnnotatedString = buildAnnotatedString {
    parts.forEachIndexed { i, part ->
        if (i > 0) withStyle(SpanStyle(color = muted)) { append(" · ") }
        val color = when (part.due) {
            Due.OVERDUE -> overdue
            Due.SOON -> soon
            else -> muted
        }
        withStyle(SpanStyle(color = color)) { append(part.text) }
    }
}

/** What a screen reader says of the parts: always the long forms. */
fun rowLabel(parts: List<RowPart>): String = parts.joinToString(", ") { it.label }

/** The sheet „Zeile …“ (ADR 0131 §1, §5): line 1 with the title, line 2 and off as rows of chips; a tap on a chip moves it. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun RowSheet(close: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val layout = RowLayout.layout.value
    val set: (RowFields) -> Unit = { next -> scope.launch { RowLayout.set(Store.get(context), next) } }
    val words = RowLayout.fields().toMap() + (RowLayout.TITLE to tr("Titel"))
    ModalBottomSheet(onDismissRequest = close, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = t.pane) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp, 0.dp, 16.dp, 24.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(tr("Felder der Zeile"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
            listOf(1 to tr("Zeile 1"), 2 to tr("Zeile 2"), 0 to tr("Aus")).forEach { (line, name) ->
                Text(name, color = t.muted, fontSize = 12.sp)
                val keys = RowLayout.keysOf(layout, line)
                FlowRow(
                    Modifier.fillMaxWidth().border(1.dp, t.line, RoundedCornerShape(8.dp)).padding(6.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    keys.forEachIndexed { index, key ->
                        RowChip(words[key] ?: key, key == RowLayout.TITLE, line == 0) { menu, dismiss ->
                            val title = key == RowLayout.TITLE
                            val moves = listOfNotNull(
                                if (line != 0 && index > 0) tr("nach links") to { RowLayout.moved(layout, key, line, index - 1) } else null,
                                if (line != 0 && index < keys.size - 1) tr("nach rechts") to { RowLayout.moved(layout, key, line, index + 1) } else null,
                                if (line != 1) tr("in Zeile 1") to { RowLayout.moved(layout, key, 1) } else null,
                                if (line != 2 && !title) tr("in Zeile 2") to { RowLayout.moved(layout, key, 2) } else null,
                                if (line != 0 && !title) tr("aus") to { RowLayout.moved(layout, key, 0) } else null,
                            )
                            DropdownMenu(expanded = menu, onDismissRequest = dismiss) {
                                moves.forEach { (label, next) ->
                                    DropdownMenuItem(text = { Text(label) }, onClick = { dismiss(); set(next()) })
                                }
                            }
                        }
                    }
                }
            }
            Row(Modifier.fillMaxWidth().padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(tr("Kurzform mit Symbolen"), color = t.fg, fontSize = 15.sp, modifier = Modifier.weight(1f))
                Switch(checked = layout.short, onCheckedChange = { set(layout.copy(short = it)) })
            }
            Row(Modifier.fillMaxWidth()) {
                Spacer(Modifier.weight(1f))
                TextButton(onClick = { scope.launch { val s = Store.get(context); RowLayout.set(s, s.rowFieldsDefault()) } }) {
                    Text(tr("Vorgabe"), color = t.muted)
                }
                TextButton(onClick = close) { Text(tr("Fertig"), color = t.accent, fontWeight = FontWeight.SemiBold) }
            }
        }
    }
}

/** One chip of the sheet; a tap opens its menu, which `menu` draws. */
@Composable
private fun RowChip(word: String, title: Boolean, off: Boolean, menu: @Composable (Boolean, () -> Unit) -> Unit) {
    val t = LocalTokens.current
    var open by remember { mutableStateOf(false) }
    Box {
        Box(
            Modifier.border(1.dp, if (title) t.accent else t.line, RoundedCornerShape(14.dp))
                .background(if (title) t.select else t.pane, RoundedCornerShape(14.dp))
                .clickable { open = true }
                .semantics { contentDescription = word }
                .padding(horizontal = 12.dp, vertical = 6.dp),
        ) {
            Text(word, color = if (off) t.muted else t.fg, fontSize = 14.sp, fontWeight = if (title) FontWeight.SemiBold else FontWeight.Normal)
        }
        menu(open) { open = false }
    }
}
