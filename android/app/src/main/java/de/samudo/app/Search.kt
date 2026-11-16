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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.BarEntry
import de.samudo.app.mobile.PerspectiveRow
import de.samudo.app.mobile.SearchChip
import de.samudo.app.mobile.SearchOffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** The quick filters of an empty field (ADR 0103 §3): label, kind, value. */
private val QUICK get() = listOf(
    Triple(tr("Markiert"), "is", "markiert"),
    Triple(tr("Heute fällig"), "due", "heute"),
    Triple(tr("Überfällig"), "due", tr("überfällig")),
    Triple(tr("Ohne Projekt"), "project", "inbox"),
    Triple(tr("Heute erledigt"), "completed", "heute"),
)

/** The word the cursor stands at the end of, and where it starts. */
private fun wordAt(field: TextFieldValue): Pair<Int, String> {
    val before = field.text.substring(0, field.selection.start.coerceIn(0, field.text.length))
    val start = before.indexOfLast { it.isWhitespace() } + 1
    return start to before.substring(start)
}

/**
 * Search in the search language (ADR 0042, ADR 0103 §3): recognised words
 * stand as chips in the field, the list under it offers what fits the word
 * at the cursor, and an empty field shows quick filters and the last
 * searches and the perspectives. The search is the chips' words and the
 * typed rest, one text; „Als Perspektive“ saves it as one and opens it
 * (ADR 0103 §5).
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SearchScreen(
    tick: Int,
    tag: String?,
    onTag: (String?) -> Unit,
    query: String,
    onQuery: (String) -> Unit,
    changed: () -> Unit,
    open: (String) -> Unit,
    openPerspective: (BarEntry) -> Unit,
) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val samu = remember { Store.get(context) }
    val focus = remember { FocusRequester() }
    // Back in the tab, the last search comes back as chips and text.
    val restored = remember {
        if (query.isBlank()) null else runCatching { samu.searchSplit("$query ") }.getOrNull()
    }
    var chips by remember { mutableStateOf(restored?.chips ?: emptyList()) }
    var field by remember {
        val rest = restored?.rest ?: query
        mutableStateOf(TextFieldValue(rest, TextRange(rest.length)))
    }
    var offers by remember { mutableStateOf<List<SearchOffer>>(emptyList()) }
    var recent by remember { mutableStateOf<List<String>>(emptyList()) }
    var saving by remember { mutableStateOf(false) }
    val (perspectives, _) = loaded(tick, "perspectives") { it.perspectives() }
    val text = (chips.map { it.word } + field.text.trim()).filter { it.isNotEmpty() }.joinToString(" ")
    val blank = text.isEmpty()

    // Finished words that are chips leave the text for the chips.
    val type: (TextFieldValue) -> Unit = { next ->
        val split = if (next.text.any { it.isWhitespace() }) runCatching { samu.searchSplit(next.text) }.getOrNull() else null
        if (split != null && split.chips.isNotEmpty()) {
            chips = chips + split.chips
            field = TextFieldValue(split.rest, TextRange(split.rest.length))
        } else {
            field = next
        }
    }
    val restore: (String) -> Unit = { line ->
        runCatching { samu.searchSplit("$line ") }.getOrNull()?.let { split ->
            chips = split.chips
            field = TextFieldValue(split.rest, TextRange(split.rest.length))
        }
    }
    val keep: () -> Unit = {
        runCatching { samu.rememberSearch(text) }
        recent = runCatching { samu.recentSearches() }.getOrDefault(recent)
    }
    val take: (SearchOffer) -> Unit = { offer ->
        val (start, _) = wordAt(field)
        val end = field.selection.start.coerceIn(start, field.text.length)
        val chip = offer.chip
        val put = if (chip != null) "" else offer.insert.orEmpty()
        if (chip != null) chips = chips + chip
        val next = field.text.substring(0, start) + put + field.text.substring(end)
        field = TextFieldValue(next, TextRange(start + put.length))
        focus.requestFocus()
    }

    LaunchedEffect(Unit) {
        if (query.isEmpty()) focus.requestFocus()
        recent = withContext(Dispatchers.IO) { runCatching { samu.recentSearches() }.getOrDefault(emptyList()) }
    }
    // A short pause before searching, so each key does not load anew.
    LaunchedEffect(text) {
        delay(200)
        onQuery(text)
    }
    val word = wordAt(field).second
    LaunchedEffect(word) {
        offers = if (word.isEmpty()) emptyList() else withContext(Dispatchers.IO) {
            runCatching { samu.searchOffers(word) }.getOrDefault(emptyList())
        }
    }

    Column(Modifier.fillMaxSize()) {
        FlowRow(
            Modifier.fillMaxWidth().padding(16.dp, 0.dp, 16.dp, 8.dp)
                .border(1.dp, t.border, RoundedCornerShape(6.dp))
                .clickable { focus.requestFocus() }
                .padding(10.dp, 10.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            chips.forEachIndexed { i, chip ->
                FieldChip(
                    chip,
                    negate = {
                        runCatching { samu.searchChip(chip.kind, chip.value, !chip.not) }.getOrNull()?.let { flipped ->
                            chips = chips.toMutableList().also { it[i] = flipped }
                        }
                    },
                    remove = { chips = chips.filterIndexed { j, _ -> j != i } },
                )
            }
            Box(Modifier.widthIn(min = 80.dp).weight(1f).align(Alignment.CenterVertically)) {
                if (blank) Text(tr("Text, @Tag, #Projekt, fällig:heute …"), color = t.muted, fontSize = 16.sp)
                BasicTextField(
                    field, type,
                    singleLine = true,
                    textStyle = TextStyle(color = t.fg, fontSize = 16.sp),
                    cursorBrush = SolidColor(t.accent),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(onSearch = { keep() }),
                    modifier = Modifier.fillMaxWidth().focusRequester(focus),
                )
            }
        }
        if (offers.isNotEmpty()) {
            FlowRow(
                Modifier.fillMaxWidth().padding(16.dp, 0.dp, 16.dp, 8.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                offers.forEach { offer ->
                    Pill(offer.label, t.accent) { take(offer) }
                }
            }
        }
        // The tags of the list stand under the field (ADR 0103 §3).
        TagChips(tick, tag, onTag)
        if (blank) {
            EmptySearch(recent, perspectives.orEmpty(), add = { chip -> chips = chips + chip }, restore = restore, open = openPerspective)
        } else {
            Row(Modifier.fillMaxWidth().padding(16.dp, 0.dp, 16.dp, 8.dp)) {
                Spacer(Modifier.weight(1f))
                Pill(tr("☆ Als Perspektive"), t.accent) { saving = true }
            }
            // The query after the pause, so each key does not load anew.
            TaskList(
                tick, query to tag, { it.search(query, tag) }, changed,
                empty = tr("Nichts gefunden."),
                open = { id -> keep(); open(id) },
            )
        }
    }
    if (saving) {
        SavePerspectiveDialog(text, close = { saving = false }) { entry ->
            saving = false
            keep()
            changed()
            openPerspective(entry)
        }
    }
}

/**
 * Name and emoji for a search saved as a perspective (ADR 0103 §5); made
 * through the outbox, `made` gets it as an entry to open. A name taken
 * shows under the field.
 */
@Composable
private fun SavePerspectiveDialog(search: String, close: () -> Unit, made: (BarEntry) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val t = LocalTokens.current
    var name by remember { mutableStateOf("") }
    var icon by remember { mutableStateOf("") }
    var problem by remember { mutableStateOf<String?>(null) }
    AlertDialog(
        onDismissRequest = close,
        title = { Text(tr("Als Perspektive sichern")) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(search, color = t.muted, fontSize = 13.sp)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        icon, { icon = it; problem = null }, singleLine = true,
                        label = { Text("Emoji") }, modifier = Modifier.width(88.dp),
                    )
                    Spacer(Modifier.width(8.dp))
                    OutlinedTextField(
                        name, { name = it; problem = null }, singleLine = true,
                        label = { Text("Name") }, modifier = Modifier.weight(1f),
                    )
                }
                problem?.let { Text(it, color = t.overdue, fontSize = 12.sp) }
            }
        },
        confirmButton = {
            TextButton(enabled = name.isNotBlank(), onClick = {
                scope.launch {
                    try {
                        val emoji = icon.trim().ifEmpty { null }
                        val id = withContext(Dispatchers.IO) { Store.get(context).createPerspective(name, emoji, search) }
                        made(BarEntry("perspective", id, name.trim(), emoji, false))
                    } catch (e: Exception) {
                        problem = said(e)
                    }
                }
            }) { Text(tr("Sichern")) }
        },
        dismissButton = { TextButton(onClick = close) { Text(tr("Abbrechen")) } },
    )
}

/** A chip's colours by its kind: background and text. */
@Composable
private fun chipColors(chip: SearchChip): Pair<Color, Color> {
    val t = LocalTokens.current
    return when (chip.kind) {
        "project" -> t.select to t.accent
        "tag" -> t.okBg to t.ok
        "due", "planned", "defer" -> t.soonBg to t.soon
        else -> t.rowline to t.fg
    }
}

/**
 * The head of a perspective: its chips and its words, to look at
 * (ADR 0102 §4); changing them stays in the web and the CLI.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun PerspectiveHead(tick: Int, id: String) {
    val t = LocalTokens.current
    val (list, _) = loaded(tick, "perspectives") { it.perspectives() }
    val p = list?.find { it.id == id } ?: return
    if (p.chips.isEmpty() && p.text.isBlank()) return
    FlowRow(
        Modifier.fillMaxWidth().padding(16.dp, 0.dp, 16.dp, 8.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        p.chips.forEach { chip ->
            val (bg, fg) = chipColors(chip)
            Text(chip.label, color = fg, fontSize = 13.sp,
                modifier = Modifier.background(bg, RoundedCornerShape(999.dp)).padding(10.dp, 4.dp))
        }
        if (p.text.isNotBlank()) {
            Text("„${p.text}“", color = t.muted, fontSize = 13.sp, modifier = Modifier.padding(4.dp, 4.dp))
        }
    }
}

/** A chip in the field: a tap negates it, × takes it out. */
@Composable
private fun FieldChip(chip: SearchChip, negate: () -> Unit, remove: () -> Unit) {
    val (bg, fg) = chipColors(chip)
    Row(
        Modifier.background(bg, RoundedCornerShape(999.dp)),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(chip.label, color = fg, fontSize = 13.sp,
            modifier = Modifier.clickable(onClick = negate).padding(10.dp, 4.dp, 4.dp, 4.dp))
        Text("×", color = fg, fontSize = 15.sp,
            modifier = Modifier.clickable(onClick = remove).padding(4.dp, 2.dp, 10.dp, 2.dp))
    }
}

/** An outlined pill to tap, as the offers in the capture sheet. */
@Composable
private fun Pill(label: String, color: Color, onClick: () -> Unit) {
    val t = LocalTokens.current
    Text(label, color = color, fontSize = 13.sp,
        modifier = Modifier
            .border(1.dp, if (color == t.fg) t.border else color, RoundedCornerShape(999.dp))
            .clickable(onClick = onClick)
            .padding(12.dp, 6.dp))
}

/** An empty field: quick filters, the last searches of this phone and the perspectives. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun EmptySearch(
    recent: List<String>,
    perspectives: List<PerspectiveRow>,
    add: (SearchChip) -> Unit,
    restore: (String) -> Unit,
    open: (BarEntry) -> Unit,
) {
    val context = LocalContext.current
    val t = LocalTokens.current
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp, 8.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text("Filter", color = t.muted, fontSize = 13.sp)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            QUICK.forEach { (label, kind, value) ->
                Pill(label, t.fg) {
                    runCatching { Store.get(context).searchChip(kind, value, false) }.getOrNull()?.let(add)
                }
            }
        }
        if (recent.isNotEmpty()) {
            Text(tr("Zuletzt"), color = t.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 8.dp))
            recent.forEach { line ->
                Text("⌕  $line", color = t.fg, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clickable { restore(line) }.padding(0.dp, 8.dp))
            }
        }
        if (perspectives.isNotEmpty()) {
            Text(tr("Perspektiven"), color = t.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 8.dp))
            perspectives.forEach { p ->
                val entry = BarEntry("perspective", p.id, p.title, p.icon, false)
                Row(
                    Modifier.fillMaxWidth().clickable { open(entry) }.padding(0.dp, 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(Modifier.width(28.dp), contentAlignment = Alignment.Center) { BarSymbol(entry, t.muted) }
                    Spacer(Modifier.width(8.dp))
                    Text(p.title, color = t.fg, fontSize = 15.sp)
                }
            }
        }
    }
}
