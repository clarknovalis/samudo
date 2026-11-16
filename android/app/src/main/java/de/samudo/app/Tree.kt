// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.PlaceRow
import de.samudo.app.mobile.Samu
import de.samudo.app.mobile.TreeRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * A line being typed in the tree (ADR 0085 §2, §3): a new project or folder
 * in the folder `at` (null for the top), or a new name for `id`.
 */
data class TreeDraft(val kind: String, val at: String? = null, val id: String? = null, val title: String = "") {
    val renaming get() = id != null
}

/** The folder a row lies in, from the rows above it; null at the top. */
private fun parentOf(rows: List<TreeRow>, row: TreeRow): String? {
    if (row.depth == 0u) return null
    val i = rows.indexOf(row)
    return (i - 1 downTo 0).map { rows[it] }.firstOrNull { it.depth == row.depth - 1u && it.kind == "folder" }?.id
}

/** Where a new line goes in the rows: last among its siblings of its kind. */
private fun draftIndex(rows: List<TreeRow>, draft: TreeDraft): Int {
    var start = 0
    var end = rows.size
    var depth = 0u
    if (draft.at != null) {
        val i = rows.indexOfFirst { it.id == draft.at }
        if (i < 0) return rows.size
        depth = rows[i].depth + 1u
        start = i + 1
        end = (start until rows.size).firstOrNull { rows[it].depth <= rows[i].depth } ?: rows.size
    }
    if (draft.kind == "project") return end
    // A folder goes before the first project at its level.
    return (start until end).firstOrNull { rows[it].depth == depth && rows[it].kind == "project" } ?: end
}

/**
 * The tab „Projekte“ as a tree (ADR 0085): a tap opens a project or folds a
 * folder, a long press opens the row's menu.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ProjectTree(
    tick: Int,
    draft: TreeDraft?,
    setDraft: (TreeDraft?) -> Unit,
    changed: () -> Unit,
    open: (String, String) -> Unit,
    settings: (String) -> Unit,
) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    // Folding is the phone's own: it reloads without a sync.
    var local by remember { mutableIntStateOf(0) }
    var rows by remember { mutableStateOf<List<TreeRow>>(emptyList()) }
    var menu by remember { mutableStateOf<String?>(null) }
    var moving by remember { mutableStateOf<TreeRow?>(null) }
    LaunchedEffect(tick, local) {
        rows = withContext(Dispatchers.IO) { runCatching { Store.get(context).projectTree() }.getOrDefault(emptyList()) }
    }
    // Drafting into a folded folder unfolds it first.
    LaunchedEffect(draft?.at) {
        val at = draft?.at ?: return@LaunchedEffect
        if (rows.any { it.id == at && it.folded }) {
            withContext(Dispatchers.IO) { runCatching { Store.get(context).setFolded(at, false) } }
            local++
        }
    }
    BackHandler(enabled = draft != null) { setDraft(null) }

    fun act(sync: Boolean = true, action: Samu.() -> Unit) {
        scope.launch {
            val error = withContext(Dispatchers.IO) { runCatching { Store.get(context).action() }.exceptionOrNull() }
            if (error != null) Toast.makeText(context, said(error), Toast.LENGTH_SHORT).show()
            if (sync) changed() else local++
        }
    }
    fun commit(d: TreeDraft, name: String) {
        setDraft(null)
        val typed = name.trim()
        if (typed.isEmpty() || (d.renaming && typed == d.title)) return
        act {
            when {
                d.renaming -> renamePlace(d.id!!, typed)
                d.kind == "folder" -> createFolder(typed, d.at)
                else -> createProject(typed, d.at)
            }
        }
    }

    val lines: List<Any> = buildList {
        addAll(rows)
        if (draft != null) {
            if (draft.renaming) {
                val i = indexOfFirst { (it as? TreeRow)?.id == draft.id }
                if (i >= 0) set(i, draft)
            } else {
                add(draftIndex(rows, draft), draft)
            }
        }
    }
    // The line being typed comes into view first, else it is never laid
    // out and gets no focus.
    val list = rememberLazyListState()
    val draftAt = lines.indexOfFirst { it is TreeDraft }
    LaunchedEffect(draft, draftAt) { if (draftAt >= 0) list.animateScrollToItem(maxOf(0, draftAt - 2)) }
    LazyColumn(Modifier.fillMaxSize().imePadding(), state = list) {
        if (rows.isEmpty() && draft == null) {
            item { Text(tr("Noch keine Projekte. „+“ oben legt eins an."), color = t.muted, fontSize = 14.sp, modifier = Modifier.padding(16.dp)) }
        }
        items(lines, key = { if (it is TreeRow) it.id else "draft" }) { line ->
            when (line) {
                is TreeDraft -> {
                    val depth = if (line.renaming) {
                        rows.firstOrNull { it.id == line.id }?.depth?.toInt() ?: 0
                    } else {
                        line.at?.let { at -> rows.firstOrNull { it.id == at }?.depth?.toInt()?.plus(1) } ?: 0
                    }
                    DraftLine(line, depth, commit = { commit(line, it) }, cancel = { setDraft(null) })
                }
                is TreeRow -> Box {
                    TreeLine(
                        line,
                        tap = {
                            if (line.kind == "folder") {
                                act(sync = false) { setFolded(line.id, !line.folded) }
                            } else {
                                open(line.id, listOfNotNull(line.icon, line.title).joinToString(" "))
                            }
                        },
                        longPress = { menu = line.id },
                    )
                    DropdownMenu(expanded = menu == line.id, onDismissRequest = { menu = null }) {
                        @Composable
                        fun entry(text: String, action: () -> Unit) =
                            DropdownMenuItem(text = { Text(text) }, onClick = { menu = null; action() })
                        if (line.kind == "folder") {
                            entry(tr("Neues Projekt hier")) { setDraft(TreeDraft("project", at = line.id)) }
                            entry(tr("Neuer Ordner hier")) { setDraft(TreeDraft("folder", at = line.id)) }
                        }
                        // ADR 0088 §7: „Geprüft“ on a project due for review.
                        if (line.reviewDue) entry(tr("Geprüft")) { act { markReviewed(line.id) } }
                        if (line.canUp) entry(tr("Nach oben")) { act { stepPlace(line.id, true) } }
                        if (line.canDown) entry(tr("Nach unten")) { act { stepPlace(line.id, false) } }
                        entry(tr("Verschieben nach …")) { moving = line }
                        entry(tr("Umbenennen")) { setDraft(TreeDraft(line.kind, id = line.id, title = line.title)) }
                        entry(tr("In den Einstellungen")) { settings(line.id) }
                    }
                }
            }
            Box(Modifier.fillMaxWidth().heightIn(min = 1.dp, max = 1.dp).background(t.rowline))
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
    moving?.let { row ->
        PlaceMovePicker(row, parentOf(rows, row), close = { moving = null }) { target ->
            moving = null
            act { movePlace(row.id, target) }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun TreeLine(row: TreeRow, tap: () -> Unit, longPress: () -> Unit) {
    val t = LocalTokens.current
    val folder = row.kind == "folder"
    Row(
        Modifier.fillMaxWidth().background(t.pane)
            .combinedClickable(onClick = tap, onLongClick = longPress)
            .heightIn(min = 52.dp)
            .padding(start = (16 + 18 * row.depth.toInt()).dp, end = 16.dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // Every row keeps the triangle's room, so a project sits below its
        // folder's title, not beside it.
        Text(if (!folder) "" else if (row.folded) "▸" else "▾", color = t.muted, fontSize = 14.sp, modifier = Modifier.width(20.dp))
        Column(Modifier.weight(1f)) {
            // The emoji before the title (ADR 0059 §4), ADR 0069 §9's quiet mark.
            Text(
                listOfNotNull(row.icon, row.title).joinToString(" ") + (row.ownScopeTitle?.let { " ◌ $it" } ?: if (row.scopeOnly) " ◌" else "") + if (row.onHold) tr(" · ruht") else "",
                color = if (row.onHold) t.muted else t.fg,
                fontWeight = if (folder) FontWeight.SemiBold else FontWeight.Normal,
                fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            if (row.stuck) Text(tr("keine nächste Aktion"), color = t.wait, fontSize = 13.sp)
            if (row.reviewDue) Text(tr("Review fällig"), color = t.accent, fontSize = 13.sp)
        }
        // A folder sums up what it hides (ADR 0085 §7).
        if (!folder || (row.folded && row.available > 0u)) {
            Text("${row.available}", color = t.muted, fontSize = 14.sp)
        }
    }
}

/** The line a name is typed in: Fertig takes it, empty and unfocused or Zurück cancels. */
@Composable
private fun DraftLine(draft: TreeDraft, depth: Int, commit: (String) -> Unit, cancel: () -> Unit) {
    val t = LocalTokens.current
    var text by remember(draft) { mutableStateOf(draft.title) }
    var focused by remember(draft) { mutableStateOf(false) }
    val requester = remember { FocusRequester() }
    LaunchedEffect(draft) { requester.requestFocus() }
    OutlinedTextField(
        text, { text = it }, singleLine = true,
        placeholder = { Text(if (draft.kind == "folder") tr("Neuer Ordner") else tr("Neues Projekt")) },
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { commit(text) }),
        modifier = Modifier.fillMaxWidth().background(t.pane)
            .padding(start = (28 + 18 * depth).dp, end = 16.dp, top = 4.dp, bottom = 4.dp)
            .focusRequester(requester)
            .onFocusChanged { state ->
                if (state.isFocused) focused = true
                else if (focused && text.isBlank()) cancel()
            },
    )
}

/** „Verschieben nach …“ (ADR 0085 §5): the top and the folders, never itself or below it. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PlaceMovePicker(row: TreeRow, current: String?, close: () -> Unit, pick: (String?) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var targets by remember { mutableStateOf<List<PlaceRow>>(emptyList()) }
    var query by remember { mutableStateOf("") }
    LaunchedEffect(row.id) {
        targets = withContext(Dispatchers.IO) { runCatching { Store.get(context).placeTargets(row.id) }.getOrDefault(emptyList()) }
    }
    ModalBottomSheet(onDismissRequest = close, sheetState = sheet, containerColor = t.pane) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
            Text(tr("„{title}“ verschieben nach", "title" to row.title), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
            OutlinedTextField(
                query, { query = it }, singleLine = true,
                placeholder = { Text(tr("Ordner suchen")) },
                modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
            )
            val words = query.trim().lowercase()
            val shown = if (words.isEmpty()) targets else targets.filter { it.title.lowercase().contains(words) }
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 480.dp)) {
                if (words.isEmpty()) {
                    item(key = "top") { Target(tr("Oben"), 0, current == null) { pick(null) } }
                }
                items(shown, key = { it.id }) { p ->
                    Target("▸ " + p.title, if (words.isEmpty()) p.depth.toInt() else 0, p.id == current) { pick(p.id) }
                }
            }
        }
    }
}

@Composable
private fun Target(label: String, depth: Int, here: Boolean, pick: () -> Unit) {
    val t = LocalTokens.current
    // Where it is now: marked, not a choice.
    Text(
        label + if (here) "  ✓" else "", color = if (here) t.muted else t.fg, fontSize = 15.sp,
        fontWeight = if (here) FontWeight.SemiBold else FontWeight.Normal,
        modifier = Modifier.fillMaxWidth().then(if (here) Modifier else Modifier.clickable(onClick = pick))
            .padding(start = (8 + 18 * depth).dp, top = 12.dp, bottom = 12.dp),
    )
}
