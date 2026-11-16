// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.BarEntry

/**
 * The symbols of the bar (ADR 0102 §5): the strokes of the web's sidebar
 * (`web/lib/sidebar.js`, `VIEWS` and `FUNNEL`) on a 24 grid, so web and
 * app look alike. Verfügbar, Projekte, Suche, Habits and a project without
 * an emoji have no symbol there; theirs are drawn in the same manner.
 */
object Glyphs {
    private val paths: Map<String, List<String>> = mapOf(
        "inbox" to listOf("M3 13 5.5 5h13L21 13v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z", "M3 13h5l1.5 3h5l1.5-3h5"),
        "available" to listOf("M12 3a9 9 0 1 0 0 18 9 9 0 1 0 0-18z", "M8 12.5l2.7 2.7L16.2 9.6"),
        "today" to listOf(
            "M12 8a4 4 0 1 0 0 8 4 4 0 1 0 0-8z",
            "M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
        ),
        "forecast" to listOf("M3 17l6-6 4 4 8-8", "M15 7h6v6"),
        "flagged" to listOf("M5 21V4", "M5 4h12l-2.5 4.5L17 13H5"),
        "stuck" to listOf("M12 3.5 21.5 20h-19z", "M12 10v4.5", "M12 17.3h.01"),
        "review" to listOf(
            "M2.5 12s3.5-6.5 9.5-6.5 9.5 6.5 9.5 6.5-3.5 6.5-9.5 6.5S2.5 12 2.5 12z",
            "M12 9a3 3 0 1 0 0 6 3 3 0 1 0 0-6z",
        ),
        // The web's „Alle Projekte“.
        "projects" to listOf("M12 3 21 8l-9 5-9-5z", "M3 12.5l9 5 9-5", "M3 16.5l9 5 9-5"),
        "search" to listOf("M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 1 0 0-13z", "M15.5 15.5 20 20"),
        "habits" to listOf("M17 2.5l3 3-3 3", "M4 11V9.5a4 4 0 0 1 4-4h12", "M7 21.5l-3-3 3-3", "M20 13v1.5a4 4 0 0 1-4 4H4"),
        "funnel" to listOf("M3 5h18l-7 8.5V19l-4 2v-7.5z"),
        "project" to listOf("M9 6h11M9 12h11M9 18h11", "M4.5 6h.01M4.5 12h.01M4.5 18h.01"),
    )

    private val built = HashMap<String, ImageVector>()

    /** The symbol by its name; a name without one has none. */
    fun of(name: String): ImageVector? {
        built[name]?.let { return it }
        val strokes = paths[name] ?: return null
        val builder = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f)
        strokes.forEach { d ->
            builder.addPath(
                PathParser().parsePathString(d).toNodes(),
                fill = null,
                stroke = SolidColor(Color.Black),
                strokeLineWidth = 1.7f,
                strokeLineCap = StrokeCap.Round,
                strokeLineJoin = StrokeJoin.Round,
            )
        }
        return builder.build().also { built[name] = it }
    }
}

/** How many entries the bar shows at once (ADR 0102 §2). */
private const val SHOWN = 5

/** Whether two entries are the same: by kind and key, not by title. */
fun BarEntry.same(other: BarEntry?): Boolean = other != null && kind == other.kind && key == other.key

/**
 * The bar at the bottom (ADR 0102 §1, §2): its entries, five at a time,
 * the rest by swiping sideways. A dot at the edge says it goes on; the
 * active entry scrolls into the visible part.
 */
@Composable
fun Bar(entries: List<BarEntry>, active: BarEntry?, pick: (BarEntry) -> Unit) {
    val t = LocalTokens.current
    val state = rememberLazyListState()
    val index = entries.indexOfFirst { it.same(active) }
    LaunchedEffect(index, entries.size) {
        if (index < 0) return@LaunchedEffect
        // Only a fully shown entry counts as visible; before the first
        // layout nothing does, and the first five need no scrolling.
        val info = state.layoutInfo
        val whole = info.visibleItemsInfo.filter { it.offset >= 0 && it.offset + it.size <= info.viewportEndOffset }
        if (whole.any { it.index == index } || (whole.isEmpty() && index < SHOWN)) return@LaunchedEffect
        val first = whole.firstOrNull()?.index ?: 0
        state.animateScrollToItem(if (index < first) index else (index - SHOWN + 1).coerceAtLeast(0))
    }
    Box(Modifier.fillMaxWidth().background(t.side).windowInsetsPadding(WindowInsets.navigationBars)) {
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val width = maxWidth / entries.size.coerceIn(1, SHOWN)
            LazyRow(state = state, modifier = Modifier.fillMaxWidth()) {
                itemsIndexed(entries, key = { _, e -> e.kind + ":" + e.key }) { _, entry ->
                    BarItem(entry, entry.same(active), Modifier.width(width)) { pick(entry) }
                }
            }
        }
        if (state.canScrollForward) EdgeDot(Modifier.align(Alignment.CenterEnd).padding(end = 3.dp))
        if (state.canScrollBackward) EdgeDot(Modifier.align(Alignment.CenterStart).padding(start = 3.dp))
    }
}

@Composable
private fun EdgeDot(modifier: Modifier) {
    val t = LocalTokens.current
    Box(modifier.size(5.dp).background(t.muted, CircleShape))
}

@Composable
private fun BarItem(entry: BarEntry, selected: Boolean, modifier: Modifier, tap: () -> Unit) {
    val t = LocalTokens.current
    val color = if (selected) t.accent else t.fg
    Column(
        modifier.clickable(onClick = tap).padding(top = 10.dp, bottom = 10.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // The pill behind the active symbol, as Material's navigation bar.
        Box(
            Modifier.size(56.dp, 30.dp)
                .background(if (selected) t.select else Color.Transparent, RoundedCornerShape(15.dp)),
            contentAlignment = Alignment.Center,
        ) { BarSymbol(entry, color) }
        Spacer(Modifier.height(4.dp))
        Text(
            entry.title, color = color, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
            fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
            modifier = Modifier.padding(horizontal = 4.dp),
        )
    }
}

/**
 * A view's symbol; a perspective's or project's emoji, else the funnel or
 * the project's symbol; a tag's `@` (ADR 0102 §5).
 */
@Composable
internal fun BarSymbol(entry: BarEntry, color: Color) {
    val icon = entry.icon
    if (icon != null && entry.kind != "view") {
        Text(icon, fontSize = 18.sp)
        return
    }
    val name = when (entry.kind) {
        "view" -> entry.key
        "perspective" -> "funnel"
        "project" -> "project"
        else -> null
    }
    val glyph = name?.let { Glyphs.of(it) }
    if (glyph != null) {
        Icon(glyph, contentDescription = null, tint = color, modifier = Modifier.size(22.dp))
    } else {
        Text("@", color = color, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
    }
}
