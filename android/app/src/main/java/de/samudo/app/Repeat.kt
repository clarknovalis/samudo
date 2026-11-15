package de.samudo.app

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.RepeatEdit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// A repetition on the task page (ADR 0007 §3, 85b3799a): the rule in
// words with the next date, and a sheet with the same form as the web
// inspector. samu-mobile takes the rule apart and puts it together.

private val FREQS get() = listOf(
    Triple("DAILY", tr("täglich"), tr("Tag") to tr("Tage")),
    Triple("WEEKLY", tr("wöchentlich"), tr("Woche") to tr("Wochen")),
    Triple("MONTHLY", tr("monatlich"), tr("Monat") to tr("Monate")),
    Triple("YEARLY", tr("jährlich"), tr("Jahr") to tr("Jahre")),
)
private val DAYS get() = listOf("MO" to tr("Mo"), "TU" to tr("Di"), "WE" to tr("Mi"), "TH" to tr("Do"), "FR" to tr("Fr"), "SA" to tr("Sa"), "SU" to tr("So"))
private val MODES get() = listOf(
    "fixed" to tr("fester Plan"),
    "due_after_completion" to tr("Fällig ab Erledigung"),
    "defer_after_completion" to tr("Start ab Erledigung"),
)

/** "alle 2 Wochen Mo, Do · Fällig ab Erledigung · 5 Mal" */
fun repeatText(r: RepeatEdit): String {
    val freq = FREQS.find { it.first == r.freq }
    var text = when {
        freq == null -> r.rule
        r.interval > 1u -> tr("alle {interval} {second}", "interval" to r.interval, "second" to freq.third.second)
        else -> freq.second
    }
    if (r.byday.isNotEmpty()) text += " " + r.byday.joinToString(", ") { d -> DAYS.find { it.first == d }?.second ?: d }
    if (freq != null && r.rest.isNotEmpty()) text += " (${r.rest.joinToString(";")})"
    val parts = mutableListOf(text)
    if (r.mode == "fixed") {
        if (r.catchUp) parts += tr("holt auf")
    } else {
        parts += MODES.find { it.first == r.mode }?.second ?: r.mode
    }
    r.until?.let { parts += tr("bis {n}", "n" to Words.day(it)) }
    r.count?.let { parts += tr("{it} Mal", "it" to it) }
    return parts.joinToString(" · ")
}

/** "nächste: fällig Do 19.11." or "endet mit dieser". */
fun nextText(r: RepeatEdit): String? = when {
    r.ended -> tr("endet mit dieser")
    r.nextDate == null -> null
    else -> {
        val which = when (r.nextField) { "defer" -> tr("Start"); "planned" -> tr("geplant"); else -> tr("fällig") }
        tr("nächste: {which} {n}", "which" to which, "n" to Words.day(r.nextDate))
    }
}

/** The line under the dates; a tap opens the form. */
@Composable
fun RepeatLine(id: String, repeat: RepeatEdit?, changed: () -> Unit) {
    val t = LocalTokens.current
    var open by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth().clickable { open = true }.padding(vertical = 8.dp), verticalAlignment = Alignment.Top) {
        Text(tr("Wiederholung"), color = t.muted, fontSize = 13.sp, modifier = Modifier.width(96.dp))
        Column(Modifier.weight(1f)) {
            Text(repeat?.let { repeatText(it) } ?: "–", color = t.fg, fontSize = 14.sp)
            repeat?.let { nextText(it) }?.let { Text(it, color = t.muted, fontSize = 12.sp) }
        }
    }
    if (open) RepeatSheet(id, repeat, changed) { open = false }
}

private fun fresh(freq: String) = RepeatEdit(
    rule = "", mode = "fixed", catchUp = false, freq = freq, interval = 1u,
    byday = emptyList(), until = null, count = null, rest = emptyList(),
    nextField = null, nextDate = null, ended = false,
)

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
private fun RepeatSheet(id: String, repeat: RepeatEdit?, changed: () -> Unit, close: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var problem by remember { mutableStateOf<String?>(null) }
    fun write(edit: RepeatEdit?) {
        scope.launch {
            problem = try {
                withContext(Dispatchers.IO) {
                    val samu = Store.get(context)
                    if (edit == null) samu.clearRepeat(id) else samu.setRepeat(id, edit)
                }
                null
            } catch (e: Exception) {
                said(e)
            }
            changed()
        }
    }
    val label: @Composable (String) -> Unit = { Text(it, color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold) }
    val chip: @Composable (String, Boolean, () -> Unit) -> Unit = { text, on, tap ->
        FilterChip(selected = on, onClick = tap, label = { Text(text, fontSize = 13.sp, color = t.fg) })
    }
    ModalBottomSheet(onDismissRequest = close, sheetState = sheet, containerColor = t.pane) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(tr("Wiederholung"), color = t.fg, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
            repeat?.let { r ->
                Text(repeatText(r), color = t.fg, fontSize = 14.sp)
                nextText(r)?.let { Text(it, color = t.muted, fontSize = 13.sp) }
            }
            val fits = repeat == null || repeat.freq != null
            if (fits) {
                label(tr("HÄUFIGKEIT"))
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    chip(tr("keine"), repeat == null) { if (repeat != null) write(null) }
                    FREQS.forEach { (code, text, _) ->
                        chip(text, repeat?.freq == code) {
                            if (repeat?.freq != code) write(repeat?.copy(freq = code) ?: fresh(code))
                        }
                    }
                }
            }
            if (repeat != null && repeat.freq != null) {
                val unit = FREQS.find { it.first == repeat.freq }?.third
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(tr("alle"), color = t.fg, fontSize = 14.sp)
                    TextButton(onClick = { if (repeat.interval > 1u) write(repeat.copy(interval = repeat.interval - 1u)) }) { Text("−") }
                    Text("${repeat.interval}", color = t.fg, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                    TextButton(onClick = { write(repeat.copy(interval = repeat.interval + 1u)) }) { Text("+") }
                    unit?.let { Text(if (repeat.interval > 1u) it.second else it.first, color = t.fg, fontSize = 14.sp) }
                }
                if (repeat.freq == "WEEKLY") {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        DAYS.forEach { (code, text) ->
                            val on = code in repeat.byday
                            chip(text, on) { write(repeat.copy(byday = if (on) repeat.byday - code else repeat.byday + code)) }
                        }
                    }
                }
            }
            if (repeat != null) {
                label(tr("MODUS"))
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    MODES.forEach { (code, text) ->
                        chip(text, repeat.mode == code) { if (repeat.mode != code) write(repeat.copy(mode = code)) }
                    }
                    if (repeat.mode == "fixed") chip(tr("aufholen"), repeat.catchUp) { write(repeat.copy(catchUp = !repeat.catchUp)) }
                }
                if (repeat.freq != null) {
                    label(tr("ENDE"))
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.Center) {
                        chip(tr("nie"), repeat.until == null && repeat.count == null) {
                            write(repeat.copy(until = null, count = null))
                        }
                        chip(repeat.until?.let { tr("bis {n}", "n" to Words.day(it)) } ?: tr("bis …"), repeat.until != null) {
                            pickDay(context, repeat.until) { day -> write(repeat.copy(until = day, count = null)) }
                        }
                        chip(repeat.count?.let { tr("nach {it} Mal", "it" to it) } ?: tr("nach … Mal"), repeat.count != null) {
                            if (repeat.count == null) write(repeat.copy(count = 5u, until = null))
                        }
                    }
                    repeat.count?.let { n ->
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(tr("nach"), color = t.fg, fontSize = 14.sp)
                            TextButton(onClick = { if (n > 1u) write(repeat.copy(count = n - 1u)) }) { Text("−") }
                            Text("$n", color = t.fg, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                            TextButton(onClick = { write(repeat.copy(count = n + 1u)) }) { Text("+") }
                            Text(tr("Mal"), color = t.fg, fontSize = 14.sp)
                        }
                    }
                }
            }
            label("RRULE")
            EditableText(repeat?.rule ?: "", tr("z. B. FREQ=MONTHLY;BYMONTHDAY=-1"), single = true) { text ->
                val typed = text.trim()
                when {
                    typed.isEmpty() -> if (repeat != null) write(null)
                    typed != repeat?.rule -> write((repeat ?: fresh("DAILY")).copy(rule = typed, freq = null))
                }
            }
            problem?.let { Text(it, color = t.overdue, fontSize = 13.sp) }
            if (repeat != null) OutlinedButton(onClick = { write(null) }) { Text(tr("Entfernen")) }
            Spacer(Modifier.heightIn(min = 32.dp))
        }
    }
}
