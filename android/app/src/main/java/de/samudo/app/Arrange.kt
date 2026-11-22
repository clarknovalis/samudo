// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.Arrange
import de.samudo.app.mobile.Show
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Anordnen (ADR 0125): group, sort and a second key for a list, as the web
 * does it (ADR 0064), and which tasks it shows (ADR 0020 §1). samu-mobile
 * arranges, filters and keeps it: a perspective's on the server, a view's
 * on this phone. [rev] is bumped on each change, so the
 * lists load again; [open] is the key whose sheet is up.
 */
object Arranging {
    val rev = mutableIntStateOf(0)
    var open by mutableStateOf<String?>(null)

    val DEFAULT = Arrange("project", "position", false, null, false)

    /** Zeigen: available, remaining or all (ADR 0020 §1). */
    fun shows() = listOf(
        "available" to tr("Verfügbar"), "remaining" to tr("Verbleibend"), "all" to tr("Alle"),
    )

    /** Gruppieren, in the web's order; „Tag“ is the same word in English. */
    fun groups() = listOf(
        "project" to tr("Projekt"), "folder" to tr("Ordner"), "tag" to "Tag", "state" to tr("Zustand"),
        "priority" to tr("Priorität"), "date" to tr("Datum"), "none" to tr("keine"),
    )

    fun sorts() = listOf(
        "position" to tr("Ordnung"), "due" to tr("Fällig"), "planned" to tr("Geplant"), "defer" to tr("Start"),
        "title" to tr("Titel"), "created" to tr("Erstellt"), "modified" to tr("Geändert"),
        "estimate" to tr("Schätzung"), "priority" to tr("Priorität"),
    )

    private fun label(list: List<Pair<String, String>>, key: String) = list.find { it.first == key }?.second ?: key
    private fun arrow(desc: Boolean) = if (desc) "↓" else "↑"

    /**
     * `⇅ Alle · Zustand · Priorität ↑, Fällig ↓`: the filter when it was
     * chosen, the grouping when there is one, the sorting.
     */
    fun summary(a: Arrange, show: Show? = null): String {
        val sorts = mutableListOf("${label(sorts(), a.sort)} ${arrow(a.desc)}")
        a.then?.let { sorts += "${label(sorts(), it)} ${arrow(a.thenDesc)}" }
        val group = if (a.group == "none") null else label(groups(), a.group)
        val chosen = show?.takeIf { !it.standard }?.let { label(shows(), it.value) }
        return "⇅ " + listOfNotNull(chosen, group, sorts.joinToString(", ")).joinToString(" · ")
    }
}

/** Arrangement and filter under the title when one is not the default; tapping opens the sheet. */
@Composable
fun ArrangeLine(tick: Int, key: String) {
    val t = LocalTokens.current
    val (found, _) = loaded(tick, key to Arranging.rev.intValue) { it.arrange(key) to it.show(key) }
    val (a, show) = found ?: return
    if (a == Arranging.DEFAULT && show?.standard != false) return
    Text(
        Arranging.summary(a, show), color = t.accent, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
        modifier = Modifier.fillMaxWidth().clickable { Arranging.open = key }.padding(16.dp, 6.dp),
    )
}

/**
 * The sheet with Gruppieren, Sortieren and „dann nach“ (ADR 0125 §3), each
 * change at once; `changed` sends a perspective's through the outbox.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ArrangeSheet(key: String, changed: () -> Unit, close: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val (loadedValue, error) = loaded(0, key to Arranging.rev.intValue) { it.arrange(key) }
    // None for a list without a choice (Verfügbar, Heute).
    val (show, _) = loaded(0, key to Arranging.rev.intValue) { it.show(key) }
    var failed by remember { mutableStateOf<String?>(null) }
    val set: (Arrange) -> Unit = { next ->
        scope.launch {
            val ok = withContext(Dispatchers.IO) {
                runCatching { Store.get(context).setArrange(key, next) }
            }
            failed = ok.exceptionOrNull()?.let { said(it) }
            Arranging.rev.intValue++
            if (key.startsWith("perspective:")) changed()
        }
    }
    val setShow: (String) -> Unit = { next ->
        scope.launch {
            val ok = withContext(Dispatchers.IO) {
                runCatching { Store.get(context).setShow(key, next) }
            }
            failed = ok.exceptionOrNull()?.let { said(it) }
            Arranging.rev.intValue++
            if (key.startsWith("perspective:")) changed()
        }
    }
    ModalBottomSheet(onDismissRequest = close, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = t.pane) {
        Column(Modifier.fillMaxWidth().padding(16.dp, 0.dp, 16.dp, 24.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(tr("Anordnen"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
            (error ?: failed)?.let { Text(it, color = t.overdue, fontSize = 13.sp) }
            val a = loadedValue ?: return@Column
            val noThen = a.sort == "position"
            show?.let { ArrangeRow(tr("Zeigen"), Arranging.shows(), it.value, setShow) }
            ArrangeRow(tr("Gruppieren"), Arranging.groups(), a.group, { set(a.copy(group = it)) })
            ArrangeRow(
                tr("Sortieren"), Arranging.sorts(), a.sort, { set(a.copy(sort = it, then = a.then.takeIf { k -> k != it })) },
                desc = a.desc, flip = { set(a.copy(desc = !a.desc)) },
            )
            ArrangeRow(
                tr("dann nach"),
                listOf("" to "—") + Arranging.sorts().filter { it.first != "position" && it.first != a.sort },
                a.then ?: "", { set(a.copy(then = it.ifEmpty { null })) },
                desc = a.thenDesc, flip = { set(a.copy(thenDesc = !a.thenDesc)) },
                enabled = !noThen, flipEnabled = !noThen && a.then != null,
                hint = if (noThen) tr("Ordnung ist eindeutig") else null,
            )
            Row(Modifier.fillMaxWidth().padding(top = 8.dp)) {
                Spacer(Modifier.weight(1f))
                // The arrangement only: what a list shows is not how it stands.
                TextButton(onClick = { set(Arranging.DEFAULT) }) { Text(tr("Zurücksetzen"), color = t.muted) }
                TextButton(onClick = close) { Text(tr("Fertig"), color = t.accent, fontWeight = FontWeight.SemiBold) }
            }
        }
    }
}

/** One line of the sheet: its name, the chosen value as a menu, and ↑/↓ where it has a direction. */
@Composable
private fun ArrangeRow(
    name: String,
    choices: List<Pair<String, String>>,
    chosen: String,
    pick: (String) -> Unit,
    desc: Boolean? = null,
    flip: () -> Unit = {},
    enabled: Boolean = true,
    flipEnabled: Boolean = enabled,
    hint: String? = null,
) {
    val t = LocalTokens.current
    var menu by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(name, color = t.fg, fontSize = 15.sp, modifier = Modifier.width(110.dp))
        Box(Modifier.weight(1f)) {
            TextButton(onClick = { menu = true }, enabled = enabled) {
                Text(
                    hint ?: (choices.find { it.first == chosen }?.second ?: chosen) + "  ▾",
                    color = if (enabled) t.accent else t.muted, fontSize = 15.sp,
                )
            }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                choices.forEach { (value, label) ->
                    DropdownMenuItem(
                        text = { Text(if (value == chosen) "✓ $label" else label) },
                        onClick = { menu = false; if (value != chosen) pick(value) },
                    )
                }
            }
        }
        if (desc != null) {
            TextButton(onClick = flip, enabled = flipEnabled) {
                Text(
                    if (desc) "↓" else "↑", fontSize = 18.sp, color = if (flipEnabled) t.fg else t.muted,
                )
            }
        }
    }
}
