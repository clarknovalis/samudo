// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshots.SnapshotStateList
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.BarEntry
import de.samudo.app.mobile.Section
import de.samudo.app.mobile.ShownBlock
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The folded headings of the app's lists (ADR 0147 §4, §5), per phone and
 * across restarts, as the web keeps them per browser. A key names the list
 * and the group, so a group folded in one list stays open in another.
 */
object Folds {
    private const val PREFS = "samu"
    private const val KEY = "folds"
    private var loaded = false
    private val folded = mutableStateListOf<String>()

    private fun load(context: Context) {
        if (loaded) return
        loaded = true
        val kept = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getStringSet(KEY, emptySet()).orEmpty()
        folded.addAll(kept)
    }

    private fun keep(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putStringSet(KEY, folded.toSet()).apply()
    }

    fun isFolded(context: Context, key: String): Boolean {
        load(context)
        return key in folded
    }

    fun toggle(context: Context, key: String) {
        load(context)
        if (!folded.remove(key)) folded.add(key)
        keep(context)
    }

    /** Long press: this one open, the other headings of the list folded. */
    fun only(context: Context, key: String, all: List<String>) {
        load(context)
        for (other in all) if (other != key && other !in folded) folded.add(other)
        folded.remove(key)
        keep(context)
    }
}

/** The fold key of a group of a list: the list, then the group. */
internal fun groupFold(list: String, index: Int, section: Section): String =
    "$list|" + (section.projectId ?: section.title ?: if (index == 0) "inbox" else "group-$index")

/**
 * A heading that folds on a tap and leaves only itself open on a long press
 * (ADR 0147 §3, §4); `trailing` stands at its right end.
 */
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
internal fun FoldHead(
    folded: Boolean,
    toggle: () -> Unit,
    only: () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable RowScope.() -> Unit,
) {
    val t = LocalTokens.current
    val haptic = LocalHapticFeedback.current
    Row(
        modifier.fillMaxWidth()
            .combinedClickable(
                onClickLabel = if (folded) tr("aufklappen") else tr("zuklappen"),
                onClick = toggle,
                onLongClick = {
                    haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress)
                    only()
                },
            ),
        verticalAlignment = Alignment.Bottom,
    ) {
        Text(if (folded) "▸" else "▾", fontSize = 12.sp, color = t.muted, modifier = Modifier.width(16.dp))
        content()
    }
}

/** The heading of a group, as [TaskList] and the blocks draw it. */
@Composable
internal fun GroupHead(section: Section, folded: Boolean, toggle: () -> Unit, only: () -> Unit) {
    val t = LocalTokens.current
    FoldHead(folded, toggle, only, Modifier.padding(16.dp, 18.dp, 16.dp, 6.dp)) {
        Text(
            (section.title ?: "Inbox").uppercase(),
            fontSize = 12.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.7.sp, color = t.muted,
        )
        // The folder with its whole path next to the project (ADR 0026 §6).
        section.folder?.let { Text("  $it", fontSize = 12.sp, color = t.muted, maxLines = 1, overflow = TextOverflow.Ellipsis) }
    }
}

/**
 * The groups of `list` with foldable headings: a heading where there is a
 * title or more than one group, the rows of an unfolded group below it.
 */
internal fun LazyListScope.groups(
    list: List<Section>,
    prefix: String,
    keys: List<String>,
    folded: (String) -> Boolean,
    toggle: (String) -> Unit,
    only: (String) -> Unit,
    changed: () -> Unit,
    open: (String) -> Unit,
) {
    list.forEachIndexed { index, section ->
        val key = keys[index]
        val headed = section.title != null || list.size > 1
        val shut = headed && folded(key)
        if (headed) {
            // Keyed by the section too: a grouping by tag shows a task in each
            // of its tags, and groups that are no project have no id (ADR 0125 §5).
            item(key = "$prefix-head-$index-$key") { GroupHead(section, shut, { toggle(key) }, { only(key) }) }
        }
        if (!shut) items(section.rows, key = { "$prefix-$index:" + it.id }) { row -> SwipeRow(row, changed) { open(row.id) } }
    }
}

/**
 * A block perspective block by block (ADR 0147): per block a heading with
 * its icon, title and count that folds; a symbol at its right opens what
 * it refers to. Below, its groups, or a note block's note, rendered and
 * editable as in the inspector. An empty block starts folded; opened, it
 * stays open while the list is shown.
 */
@Composable
fun BlockList(
    tick: Int,
    id: String,
    tag: String?,
    changed: () -> Unit,
    open: (String) -> Unit,
    go: (BarEntry) -> Unit,
    head: (@Composable () -> Unit)? = null,
) {
    val t = LocalTokens.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val (blocks, error) = loaded(tick, Triple("blocks", id, tag) to Arranging.rev.intValue) { it.blocks(id, tag) }
    error?.let { Text(it, color = t.overdue, modifier = Modifier.padding(16.dp)) }
    val list = blocks ?: return
    LaunchedEffect(list) { Picks.refresh(list.flatMap { b -> b.sections.flatMap { it.rows } }) }
    // Empty blocks opened by a tap, until the list is left (ADR 0147 §5).
    val openedEmpty: SnapshotStateList<String> = remember(id) { mutableStateListOf() }
    val blockKey = { i: Int -> "block:$id:$i" }
    val empty = { b: ShownBlock -> b.note == null && b.count == 0u && !b.missing }
    val shut = { i: Int, b: ShownBlock ->
        if (empty(b)) blockKey(i) !in openedEmpty else Folds.isFolded(context, blockKey(i))
    }
    // "Only this one open" counts the blocks with something in them.
    val heads = list.indices.filter { !empty(list[it]) }.map(blockKey)
    val toggle = { i: Int, b: ShownBlock ->
        val key = blockKey(i)
        if (empty(b)) {
            if (!openedEmpty.remove(key)) openedEmpty.add(key)
        } else {
            Folds.toggle(context, key)
        }
    }
    LazyColumn(Modifier.fillMaxSize()) {
        head?.let { item(key = "list-head") { Box(Modifier.heightIn(min = 1.dp)) { it() } } }
        if (list.isEmpty()) item(key = "empty") { Text(tr("Nichts hier."), color = t.muted, modifier = Modifier.padding(16.dp)) }
        list.forEachIndexed { i, block ->
            val folded = shut(i, block)
            item(key = "block-$i") {
                FoldHead(folded, { toggle(i, block) }, { Folds.only(context, blockKey(i), heads) }, Modifier.padding(16.dp, 22.dp, 8.dp, 8.dp)) {
                    Text(
                        listOfNotNull(block.icon, block.title).joinToString(" "),
                        fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = t.fg,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f, fill = false),
                    )
                    if (block.note == null) Text("  ${block.count}", fontSize = 13.sp, color = t.muted)
                    if (block.missing) Text("  " + tr("fehlt"), fontSize = 13.sp, color = t.overdue)
                    Spacer(Modifier.weight(1f))
                    val target = block.opens
                    val obj = block.`object`
                    if (target != null || obj != null) {
                        Text(
                            "↗", fontSize = 16.sp, color = t.muted,
                            modifier = Modifier
                                .clickable(onClickLabel = tr("öffnen")) { if (target != null) go(target) else if (obj != null) open(obj) }
                                .padding(horizontal = 10.dp, vertical = 2.dp),
                        )
                    }
                }
            }
            if (folded || block.missing) return@forEachIndexed
            val note = block.note
            val obj = block.`object`
            if (note != null) {
                item(key = "block-$i-note") {
                    Box(Modifier.padding(horizontal = 16.dp)) {
                        NoteField(
                            note,
                            { v ->
                                if (obj != null) scope.launch {
                                    withContext(Dispatchers.IO) { runCatching { Store.get(context).setNote(obj, v) } }
                                    changed()
                                }
                            },
                            open,
                            rendered = true,
                        )
                    }
                }
                return@forEachIndexed
            }
            if (block.sections.isEmpty()) {
                item(key = "block-$i-empty") { Text(tr("Nichts hier."), color = t.muted, modifier = Modifier.padding(32.dp, 4.dp, 16.dp, 8.dp)) }
                return@forEachIndexed
            }
            val keys = block.sections.mapIndexed { j, s -> groupFold(blockKey(i), j, s) }
            groups(
                block.sections, "block-$i", keys,
                folded = { Folds.isFolded(context, it) },
                toggle = { Folds.toggle(context, it) },
                only = { Folds.only(context, it, keys) },
                changed = changed, open = open,
            )
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}
