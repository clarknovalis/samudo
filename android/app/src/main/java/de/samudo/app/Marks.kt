package de.samudo.app

import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * The palette of scopes (ADR 0060 §2), as in the web's style.css: a light
 * line and a fuller dot, light and dark.
 */
private class Shade(val line: Long, val dot: Long, val darkLine: Long, val darkDot: Long)

private val SHADES = linkedMapOf(
    "violet" to Shade(0xFFB9AEE6, 0xFF9D8FD6, 0xFF5E5491, 0xFFB3A6F0),
    "blue" to Shade(0xFFA9C8E8, 0xFF7FA9D4, 0xFF3F6187, 0xFF8DB8E8),
    "teal" to Shade(0xFFA3D5D0, 0xFF4FA59C, 0xFF2F6B66, 0xFF6CC4BA),
    "green" to Shade(0xFFB5D6B8, 0xFF86B48B, 0xFF4A6E4E, 0xFF9BCB9F),
    "yellow" to Shade(0xFFEBD99A, 0xFFC9A93A, 0xFF7A6A2C, 0xFFE3C65C),
    "orange" to Shade(0xFFF1C29A, 0xFFDD8A45, 0xFF84532B, 0xFFF0A468),
    "red" to Shade(0xFFEDB0AA, 0xFFD0645A, 0xFF803A35, 0xFFF08A80),
    "pink" to Shade(0xFFEBB3D0, 0xFFCF6FA2, 0xFF7D3D5F, 0xFFE894C0),
)

val SCOPE_COLOR_NAMES get() = listOf(
    "violet" to tr("Violett"), "blue" to tr("Blau"), "teal" to tr("Petrol"), "green" to tr("Grün"),
    "yellow" to tr("Gelb"), "orange" to tr("Orange"), "red" to tr("Rot"), "pink" to tr("Rosa"),
)

/** The line of a scope's colour; null without one. */
@Composable
fun scopeLine(color: String?): Color? {
    val s = SHADES[color] ?: return null
    return Color(if (isSystemInDarkTheme()) s.darkLine else s.line)
}

/** The dot of a scope's colour; null without one. */
@Composable
fun scopeDotColor(color: String?): Color? {
    val s = SHADES[color] ?: return null
    return Color(if (isSystemInDarkTheme()) s.darkDot else s.dot)
}

/** The bar of a priority beside a task's circle (ADR 0089): A red, B orange, C yellow. */
@Composable
fun PriorityBar(priority: String?, faded: Boolean = false) {
    val c = when (priority) {
        "A" -> scopeDotColor("red")
        "B" -> scopeDotColor("orange")
        "C" -> scopeDotColor("yellow")
        else -> null
    }
    var m = Modifier.width(3.dp).height(24.dp)
    if (c != null) {
        m = m.background(c.copy(alpha = if (faded) 0.45f else 1f), RoundedCornerShape(2.dp))
            .semantics { contentDescription = tr("Priorität {priority}", "priority" to priority) }
    }
    Box(m)
}

/** A dot in a scope's colour, or nothing without one. */
@Composable
fun ScopeDot(color: String?, size: Int = 8) {
    val c = scopeDotColor(color) ?: return
    Box(Modifier.size(size.dp).background(c, CircleShape))
}

/** A row of swatches plus none (ADR 0060 §5). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ScopeColorPicker(current: String?, pick: (String?) -> Unit) {
    val t = LocalTokens.current
    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        val ring = { on: Boolean -> Modifier.size(32.dp).border(2.dp, if (on) t.fg else Color.Transparent, CircleShape) }
        Box(ring(current == null).clickable { if (current != null) pick(null) }, contentAlignment = Alignment.Center) {
            Box(Modifier.size(18.dp).border(1.dp, t.border, CircleShape))
        }
        SCOPE_COLOR_NAMES.forEach { (name, _) ->
            Box(ring(current == name).clickable { if (current != name) pick(name) }, contentAlignment = Alignment.Center) {
                ScopeDot(name, 18)
            }
        }
    }
}

/** A few emojis for folders and projects (ADR 0059 §5), as in the web. */
private val EMOJIS = listOf(
    "🏠", "💼", "💻", "🛠️", "📚", "🎓", "💰", "🏦", "🛒", "🍳", "🌱", "🌻",
    "🚗", "✈️", "🏃", "❤️", "👨‍👩‍👧", "🐾", "🎵", "🎨", "📷", "🧾", "📦", "⭐",
)

/** The icon of a folder or project: pick one, type any, or none. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun IconPicker(current: String?, pick: (String?) -> Unit) {
    val t = LocalTokens.current
    var typed by remember(current) { mutableStateOf(current.orEmpty()) }
    Column {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(typed, { typed = it }, singleLine = true, placeholder = { Text("Emoji") }, modifier = Modifier.width(110.dp))
            val clean = typed.trim()
            if (clean != current.orEmpty()) TextButton(onClick = { pick(clean.ifEmpty { null }) }) { Text(tr("Sichern")) }
            if (current != null) TextButton(onClick = { pick(null) }) { Text(tr("Kein Icon")) }
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
            EMOJIS.forEach { e ->
                val on = e == current
                Box(
                    Modifier.size(40.dp)
                        .background(if (on) t.select else Color.Transparent, RoundedCornerShape(8.dp))
                        .clickable { if (!on) pick(e) },
                    contentAlignment = Alignment.Center,
                ) { Text(e, fontSize = 20.sp) }
            }
        }
    }
}
