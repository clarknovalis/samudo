package de.samudo.app

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.zIndex
import de.samudo.app.mobile.BarEntry
import de.samudo.app.mobile.BarItem
import de.samudo.app.mobile.PerspectiveRow
import de.samudo.app.mobile.PlaceRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The height of one entry on the page, the step a drag moves by. */
private val ROW = 52.dp

/** How many projects the name search lists at most. */
private const val FOUND = 30

private fun BarEntry.isInbox() = kind == "view" && key == "inbox"

/**
 * „Leiste“ (ADR 0102 §3): the entries of this phone's bar, dragged by the
 * handle into their order, hidden by the switch, removed by ×; below,
 * „Hinzufügen“ with the fixed views, the perspectives, the projects by
 * name and the tags. Inbox stays in it and shown. Kept on this phone only,
 * never synced; each change is kept at once.
 */
@Composable
fun BarSection(tick: Int, changed: () -> Unit, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var entries by remember { mutableStateOf<List<BarEntry>>(emptyList()) }
    var views by remember { mutableStateOf<List<BarEntry>>(emptyList()) }
    var perspectives by remember { mutableStateOf<List<PerspectiveRow>>(emptyList()) }
    var places by remember { mutableStateOf<List<PlaceRow>>(emptyList()) }
    var name by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    // The entry being dragged, by kind and key, and how far from its slot.
    var dragging by remember { mutableStateOf<String?>(null) }
    var offset by remember { mutableFloatStateOf(0f) }
    LaunchedEffect(tick) {
        if (dragging != null) return@LaunchedEffect
        withContext(Dispatchers.IO) {
            val samu = Store.get(context)
            entries = runCatching { samu.bar() }.getOrDefault(emptyList())
            views = samu.barViews()
            perspectives = runCatching { samu.perspectives() }.getOrDefault(emptyList())
            places = runCatching { samu.places(false) }.getOrDefault(emptyList())
        }
    }
    val act = rememberAct(changed) { error = it }
    val save: (List<BarEntry>) -> Unit = { list ->
        error = null
        entries = list
        act { setBar(list.map { BarItem(it.kind, it.key, it.hidden) }) }
    }
    val current by rememberUpdatedState(entries)
    val step = with(LocalDensity.current) { ROW.toPx() }
    val id = { e: BarEntry -> e.kind + ":" + e.key }
    val has = { kind: String, key: String -> entries.any { it.kind == kind && it.key == key } }
    val append = { kind: String, key: String, title: String, icon: String? ->
        save(entries + BarEntry(kind, key, title, icon, false))
    }

    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Einstellungen")) } }
        item {
            Heading(tr("LEISTE"))
            Text(
                tr("Fünf stehen auf einmal unten, die übrigen erreicht man durch Wischen. Gilt nur für dieses Gerät."),
                color = t.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 4.dp, bottom = 4.dp),
            )
        }
        items(entries, key = id) { entry ->
            val mine = id(entry)
            val moving = dragging == mine
            Row(
                Modifier.fillMaxWidth().height(ROW)
                    .zIndex(if (moving) 1f else 0f)
                    .graphicsLayer { translationY = if (moving) offset else 0f }
                    .background(if (moving) t.select else t.side, RoundedCornerShape(8.dp))
                    .padding(end = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // The handle: dragged, the entry swaps with its neighbour
                // each time it passes half of it.
                Box(
                    Modifier.width(44.dp).fillMaxHeight().pointerInput(mine) {
                        detectDragGestures(
                            onDragStart = { dragging = mine; offset = 0f },
                            onDragEnd = { dragging = null; offset = 0f; save(current) },
                            onDragCancel = { dragging = null; offset = 0f; save(current) },
                        ) { change, amount ->
                            change.consume()
                            offset += amount.y
                            val list = current
                            val i = list.indexOfFirst { id(it) == mine }
                            if (i < 0) return@detectDragGestures
                            val j = when {
                                offset > step / 2 && i < list.lastIndex -> i + 1
                                offset < -step / 2 && i > 0 -> i - 1
                                else -> return@detectDragGestures
                            }
                            entries = list.toMutableList().apply { add(j, removeAt(i)) }
                            offset -= (j - i) * step
                        }
                    },
                    contentAlignment = Alignment.Center,
                ) { Text("≡", color = t.muted, fontSize = 20.sp) }
                BarSymbol(entry, if (entry.hidden) t.muted else t.fg)
                Spacer(Modifier.width(10.dp))
                Text(
                    entry.title, color = if (entry.hidden) t.muted else t.fg, fontSize = 15.sp,
                    maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
                )
                Switch(
                    checked = !entry.hidden, enabled = !entry.isInbox(),
                    onCheckedChange = { on -> save(entries.map { if (id(it) == mine) it.copy(hidden = !on) else it }) },
                )
                if (entry.isInbox()) {
                    Spacer(Modifier.width(40.dp))
                } else {
                    TextButton(onClick = { save(entries.filter { id(it) != mine }) }, modifier = Modifier.width(40.dp)) {
                        Text("×", color = t.muted, fontSize = 18.sp)
                    }
                }
            }
        }
        error?.let { item { Text(it, color = t.overdue, fontSize = 13.sp) } }

        item { Heading(tr("HINZUFÜGEN")) }
        val freeViews = views.filter { !has("view", it.key) }
        if (freeViews.isNotEmpty()) {
            item { Text(tr("Ansichten"), color = t.muted, fontSize = 12.sp) }
            items(freeViews, key = { "add:" + id(it) }) { v -> AddLine(v, 0) { append("view", v.key, v.title, null) } }
        }
        val freePerspectives = perspectives.filter { !has("perspective", it.id) }
        if (freePerspectives.isNotEmpty()) {
            item { Text(tr("Perspektiven"), color = t.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp)) }
            items(freePerspectives, key = { "add:perspective:" + it.id }) { p ->
                AddLine(BarEntry("perspective", p.id, p.title, p.icon, false), 0) { append("perspective", p.id, p.title, p.icon) }
            }
        }
        item {
            Text(tr("Projekte"), color = t.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp))
            OutlinedTextField(
                name, { name = it }, singleLine = true, placeholder = { Text(tr("Name des Projekts")) },
                modifier = Modifier.fillMaxWidth(),
            )
        }
        val words = name.trim()
        if (words.isNotEmpty()) {
            val found = places.filter { it.kind == "project" && !has("project", it.id) && it.title.contains(words, ignoreCase = true) }
            if (found.isEmpty()) item { Text(tr("Kein offenes Projekt mit diesem Namen."), color = t.muted, fontSize = 13.sp) }
            items(found.take(FOUND), key = { "add:project:" + it.id }) { p ->
                AddLine(BarEntry("project", p.id, p.title, null, false), 0) { name = ""; append("project", p.id, p.title, null) }
            }
        }
        val freeTags = places.filter { it.kind == "tag" && !has("tag", it.id) }
        if (freeTags.isNotEmpty()) {
            item { Text("Tags", color = t.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp)) }
            items(freeTags, key = { "add:tag:" + it.id }) { tag ->
                AddLine(BarEntry("tag", tag.id, tag.title, null, false), tag.depth.toInt()) { append("tag", tag.id, tag.title, null) }
            }
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}

/** One line under „Hinzufügen“: its symbol, its title, a +. */
@Composable
private fun AddLine(entry: BarEntry, depth: Int, tap: () -> Unit) {
    val t = LocalTokens.current
    Row(
        Modifier.fillMaxWidth().clickable(onClick = tap).padding(start = (12 + depth * 20).dp, top = 8.dp, bottom = 8.dp, end = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(28.dp), contentAlignment = Alignment.Center) { BarSymbol(entry, t.fg) }
        Spacer(Modifier.width(10.dp))
        Text(entry.title, color = t.fg, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        Text("+", color = t.accent, fontSize = 18.sp)
    }
}
