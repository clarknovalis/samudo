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
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.HabitDayRow
import de.samudo.app.mobile.HabitDetail
import de.samudo.app.mobile.HabitRow
import de.samudo.app.mobile.Samu
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The mark of a habit in place of the circle (ADR 0092 §2): ✓, green once
 * the period's goal is met, or +1 for one that counts; nothing to tap for
 * one to avoid, which shows "frei seit n T" beside it instead.
 */
@Composable
internal fun HabitMark(habit: HabitRow, tap: () -> Unit) {
    val t = LocalTokens.current
    if (habit.kind == "avoid") {
        Box(Modifier.size(24.dp).border(2.dp, if (habit.today == "fail") t.overdue else t.ok, CircleShape))
        return
    }
    val counts = Words.habitCounts(habit)
    Box(
        Modifier.heightIn(min = 24.dp).widthIn(min = 24.dp)
            .border(2.dp, if (habit.met) t.ok else t.muted, RoundedCornerShape(12.dp))
            .background(if (habit.met) t.okBg else Color.Transparent, RoundedCornerShape(12.dp))
            .clickable(onClick = tap)
            .padding(horizontal = 4.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            if (counts && !habit.met) "+1" else "✓",
            color = if (habit.met) t.ok else t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold,
        )
    }
}

/** Beside the title: `5/8`, "frei seit n T" and the streak. */
@Composable
internal fun HabitFigures(habit: HabitRow) {
    val t = LocalTokens.current
    Column(horizontalAlignment = Alignment.End, modifier = Modifier.padding(start = 8.dp)) {
        Words.habitCount(habit)?.let {
            val color = when {
                habit.kind == "avoid" && habit.today == "fail" -> t.overdue
                habit.met || habit.kind == "avoid" -> t.ok
                else -> t.muted
            }
            Text(it, color = color, fontSize = 12.sp)
        }
        Words.habitStreak(habit)?.let { Text(it, color = t.muted, fontSize = 12.sp) }
    }
}

/**
 * The menu of a long press on a habit (ADR 0092 §2): today done, not done
 * or relapse, a pause today, and completing, which ends the habit.
 */
@Composable
internal fun HabitMenu(id: String, habit: HabitRow, open: Boolean, close: () -> Unit, act: (Samu.() -> Unit) -> Unit) {
    val avoid = habit.kind == "avoid"
    DropdownMenu(expanded = open, onDismissRequest = close) {
        if (!avoid) {
            DropdownMenuItem(
                text = { Text(if (Words.habitCounts(habit)) tr("+1 heute") else tr("Heute geschafft")) },
                onClick = { close(); act { enterHabit(id, "done", null, null) } },
            )
        }
        DropdownMenuItem(
            text = { Text(if (avoid) tr("Rückfall heute") else tr("Heute nicht geschafft")) },
            onClick = { close(); act { enterHabit(id, "fail", null, null) } },
        )
        DropdownMenuItem(
            text = { Text(tr("Pause heute")) },
            onClick = { close(); act { enterHabit(id, "pause", null, null) } },
        )
        DropdownMenuItem(
            text = { Text(tr("Erledigen – beendet das Habit")) },
            onClick = { close(); act { complete(id) } },
        )
    }
}

/**
 * The section "Serie" of the task page (ADR 0092 §3): goal, streak,
 * longest, the rates, the running period and a calendar of 12 weeks. A tap
 * on a day opens its entries, to take one back or enter afterwards.
 */
@Composable
internal fun SeriesPart(id: String, tick: Int, act: (Samu.() -> Unit) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var detail by remember { mutableStateOf<HabitDetail?>(null) }
    var picked by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(tick, id) {
        detail = withContext(Dispatchers.IO) { runCatching { Store.get(context).habit(id) }.getOrNull() }
    }
    val d = detail ?: return
    val h = d.row
    val avoid = h.kind == "avoid"
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(tr("SERIE"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        val facts = buildList {
            add(tr("Ziel") to Words.habitGoal(h))
            if (avoid) {
                add(tr("Frei seit") to tr("{streak} Tagen", "streak" to h.streak))
                d.lastFail?.let { add(tr("Letzter Rückfall") to Words.day(it)) }
            } else {
                add(tr("Habit|Serie") to "${h.streak}")
                add(tr("Dieser Zeitraum") to "${h.amount}/${h.target}" + if (d.periodFrom != d.periodTo) " (${Words.day(d.periodFrom)} – ${Words.day(d.periodTo)})" else "")
            }
            add(tr("Längste") to "${d.longest}")
            add(tr("4 Wochen") to rate(d.recentMet, d.recentOf))
            add(tr("Dieses Jahr") to rate(d.yearMet, d.yearOf))
        }
        facts.forEach { (k, v) ->
            Row {
                Text(k, color = t.muted, fontSize = 13.sp, modifier = Modifier.width(130.dp))
                Text(v, color = t.fg, fontSize = 13.sp)
            }
        }
        HabitCalendar(d.days, d.today, picked) { picked = it }
    }
    picked?.let { day -> HabitDaySheet(id, d, day, act) { picked = null } }
}

private fun rate(met: UInt, of: UInt): String =
    if (of == 0u) "–" else "$met/$of (${(met.toInt() * 100 / of.toInt())} %)"

/** Twelve weeks from Monday, one column a week, in the web's colours. */
@Composable
private fun HabitCalendar(days: List<HabitDayRow>, today: String, picked: String?, pick: (String) -> Unit) {
    val t = LocalTokens.current
    Row(horizontalArrangement = Arrangement.spacedBy(3.dp)) {
        days.chunked(7).forEach { week ->
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                week.forEach { day ->
                    val (fill, edge) = when {
                        day.future -> Color.Transparent to Color.Transparent
                        day.state == "done" -> t.ok to t.ok
                        day.state == "partial" -> t.okBg to t.ok
                        day.state == "fail" -> t.overdue to t.overdue
                        day.state == "pause" -> t.waitBg to t.wait
                        else -> t.side to t.line
                    }
                    val outline = when (day.day) {
                        picked -> t.accent
                        today -> t.fg
                        else -> edge
                    }
                    Box(
                        Modifier.size(18.dp)
                            .background(fill, RoundedCornerShape(3.dp))
                            .border(if (day.day == picked || day.day == today) 2.dp else 1.dp, outline, RoundedCornerShape(3.dp))
                            .then(if (day.future) Modifier else Modifier.clickable { pick(day.day) }),
                    )
                }
            }
        }
    }
}

/**
 * A day's entries: take one back, or enter done, not done or a pause. For a
 * habit to avoid, "Frei ab hier" instead of done: only here, so that nobody
 * ticks one off by accident (4c20d39e).
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
private fun HabitDaySheet(id: String, d: HabitDetail, day: String, act: (Samu.() -> Unit) -> Unit, close: () -> Unit) {
    val t = LocalTokens.current
    val avoid = d.row.kind == "avoid"
    val state = d.days.firstOrNull { it.day == day }
    val entries = d.entries.filter { it.day == day || (it.status == "pause" && it.day <= day && (it.until ?: it.day) >= day) }
    ModalBottomSheet(onDismissRequest = close) {
        Column(Modifier.fillMaxWidth().padding(16.dp, 0.dp, 16.dp, 32.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(Words.day(day) + (if (day == d.today) tr(" · heute") else ""), fontWeight = FontWeight.SemiBold, fontSize = 16.sp, color = t.fg)
            state?.let { Text(Words.habitDay(it.state, avoid) + if (it.amount > 0u && !avoid) " · ${it.amount}" else "", color = t.muted, fontSize = 13.sp) }
            if (entries.isEmpty()) Text(tr("Keine Einträge."), color = t.muted, fontSize = 13.sp)
            entries.forEach { e ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(Words.habitEntry(e, avoid), color = t.fg, fontSize = 14.sp, modifier = Modifier.weight(1f))
                    TextButton(onClick = { act { removeHabitEntry(e.id) } }) { Text(tr("Löschen")) }
                }
            }
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (avoid) OutlinedButton(onClick = { act { freeFrom(id, day) } }) { Text(tr("Frei ab hier")) }
                else OutlinedButton(onClick = { act { enterHabit(id, "done", day, null) } }) { Text(if (Words.habitCounts(d.row)) "+1" else tr("Geschafft")) }
                OutlinedButton(onClick = { act { enterHabit(id, "fail", day, null) } }) { Text(if (avoid) tr("Rückfall") else tr("Nicht geschafft")) }
                OutlinedButton(onClick = { act { enterHabit(id, "pause", day, null) } }) { Text(tr("Pause")) }
            }
        }
    }
}
