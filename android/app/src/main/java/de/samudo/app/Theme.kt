package de.samudo.app

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** The tokens of the web UI (ADR 0026 §1): A light, B's colours dark. */
data class Tokens(
    val bg: Color,
    val side: Color,
    val pane: Color,
    val fg: Color,
    val muted: Color,
    val line: Color,
    val rowline: Color,
    val border: Color,
    val accent: Color,
    val onAccent: Color,
    val select: Color,
    val overdue: Color,
    val soon: Color,
    val soonBg: Color,
    val flag: Color,
    val ok: Color,
    val okBg: Color,
    val wait: Color,
    val waitBg: Color,
)

val Light = Tokens(
    bg = Color(0xFFFBFBFA), side = Color(0xFFF2F2EF), pane = Color(0xFFFFFFFF),
    fg = Color(0xFF1C1D1F), muted = Color(0xFF5F6368), line = Color(0xFFE3E3DE),
    rowline = Color(0xFFEFEFEB), border = Color(0xFFC9CCD3), accent = Color(0xFF2F5FD0),
    onAccent = Color(0xFFFFFFFF), select = Color(0xFFE2E8F7), overdue = Color(0xFFB42318),
    soon = Color(0xFF8A4B00), soonBg = Color(0xFFFFF1D6), flag = Color(0xFFC2410C),
    ok = Color(0xFF1D5B3C), okBg = Color(0xFFE6F2EC), wait = Color(0xFF8A4B00),
    waitBg = Color(0xFFFFF1D6),
)

val Dark = Tokens(
    bg = Color(0xFF16181D), side = Color(0xFF1B1E24), pane = Color(0xFF1F232B),
    fg = Color(0xFFE6E8EB), muted = Color(0xFF9AA1AC), line = Color(0xFF2A2E37),
    rowline = Color(0xFF22262E), border = Color(0xFF3A3F4A), accent = Color(0xFF7FB2FF),
    onAccent = Color(0xFF0E1420), select = Color(0xFF2A3550), overdue = Color(0xFFFF8A80),
    soon = Color(0xFFF0B45A), soonBg = Color(0xFF3A2D16), flag = Color(0xFFFF9F5A),
    ok = Color(0xFF8BD3A0), okBg = Color(0xFF1D3326), wait = Color(0xFFF0B45A),
    waitBg = Color(0xFF3A2D16),
)

val LocalTokens = staticCompositionLocalOf { Light }

@Composable
fun SamuTheme(content: @Composable () -> Unit) {
    val t = if (isSystemInDarkTheme()) Dark else Light
    val scheme = if (t == Dark) {
        darkColorScheme(
            primary = t.accent, onPrimary = t.onAccent, background = t.bg, onBackground = t.fg,
            surface = t.bg, onSurface = t.fg, surfaceContainer = t.side,
            secondaryContainer = t.select, outline = t.border, error = t.overdue,
        )
    } else {
        lightColorScheme(
            primary = t.accent, onPrimary = t.onAccent, background = t.bg, onBackground = t.fg,
            surface = t.bg, onSurface = t.fg, surfaceContainer = t.side,
            secondaryContainer = t.select, outline = t.border, error = t.overdue,
        )
    }
    CompositionLocalProvider(LocalTokens provides t) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
