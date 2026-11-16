// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.ProjectView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Stuck and Review in the bar (ADR 0102 §1), as samu-mobile computes them:
 * the projects, a tap opens one; in Review with its open tasks and
 * „Geprüft“ (ADR 0088 §3).
 */
@Composable
fun ProjectViews(
    tick: Int,
    review: Boolean,
    changed: () -> Unit,
    openProject: (String, String) -> Unit,
    open: (String) -> Unit,
) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val (projects, error) = loaded(tick, review) { if (review) it.review() else it.stuck() }
    error?.let { Text(it, color = t.overdue, modifier = Modifier.padding(16.dp)) }
    val list = projects ?: return
    if (list.isEmpty()) {
        Text(if (review) tr("Kein Review fällig.") else tr("Kein Projekt ohne nächste Aktion."), color = t.muted, modifier = Modifier.padding(16.dp))
        return
    }
    LazyColumn(Modifier.fillMaxSize()) {
        list.forEach { p ->
            item(key = "project-" + p.id) {
                ProjectHead(p, review, { openProject(p.id, listOfNotNull(p.icon, p.title).joinToString(" ")) }) {
                    scope.launch {
                        withContext(Dispatchers.IO) { runCatching { Store.get(context).markReviewed(p.id) } }
                        changed()
                    }
                }
            }
            items(p.rows, key = { p.id + "/" + it.id }) { row -> SwipeRow(row, changed) { open(row.id) } }
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}

@Composable
private fun ProjectHead(p: ProjectView, review: Boolean, tap: () -> Unit, reviewed: () -> Unit) {
    val t = LocalTokens.current
    Row(
        Modifier.fillMaxWidth().background(t.pane).clickable(onClick = tap)
            .padding(16.dp, if (review) 18.dp else 12.dp, 8.dp, if (review) 6.dp else 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                listOfNotNull(p.icon, p.title).joinToString(" "),
                color = t.fg, fontSize = 15.sp, fontWeight = if (review) FontWeight.SemiBold else FontWeight.Normal,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            val under = listOfNotNull(
                p.folder,
                p.review?.let { tr("fällig seit {it}", "it" to it) },
                if (review) p.lastReviewed?.let { tr("zuletzt {it}", "it" to it) } ?: tr("nie geprüft") else null,
            ).joinToString(" · ")
            if (under.isNotEmpty()) Text(under, color = t.muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (review) TextButton(onClick = reviewed) { Text(tr("Geprüft"), color = t.accent) }
    }
}
