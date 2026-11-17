// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.border
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilterChip
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import de.samudo.app.mobile.InspectorLayoutRow
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.CalendarRow
import de.samudo.app.mobile.ForecastSettings
import de.samudo.app.mobile.MemberRow
import de.samudo.app.mobile.Samu
import de.samudo.app.mobile.ScopeCaptureRow
import de.samudo.app.mobile.ScopeRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Runs a call into samu-mobile off the main thread, then draws again.
 * What it throws is shown by [onError], if given.
 */
@Composable
internal fun rememberAct(changed: () -> Unit, onError: (String) -> Unit = {}): (Samu.() -> Unit) -> Unit {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    return { action ->
        scope.launch {
            try {
                withContext(Dispatchers.IO) { Store.get(context).action() }
            } catch (e: Exception) {
                onError(said(e))
            }
            changed()
        }
    }
}

/** The scopes as the header shows them, loaded once for the chip and its line. */
class ScopeState(val scopes: List<ScopeRow>, val outside: UInt, val auto: Boolean) {
    val chosen: ScopeRow? get() = scopes.firstOrNull { it.chosen }
}

@Composable
fun rememberScopeState(tick: Int): ScopeState {
    val context = LocalContext.current
    var state by remember { mutableStateOf(ScopeState(emptyList(), 0u, false)) }
    LaunchedEffect(tick) {
        state = withContext(Dispatchers.IO) {
            val samu = Store.get(context)
            ScopeState(
                runCatching { samu.scopes() }.getOrDefault(emptyList()),
                runCatching { samu.outsideDue() }.getOrDefault(0u),
                // Following the schedule (ADR 0043 §7).
                runCatching { samu.scopeAuto() }.getOrDefault(false),
            )
        }
    }
    return state
}

/**
 * The scope as a chip in the title (ADR 0035 §7, ADR 0102 §6): always
 * visible, a menu to switch, and how much is due outside it, which takes
 * the scope off when tapped.
 */
@Composable
fun ScopeChip(state: ScopeState, changed: () -> Unit, edit: () -> Unit) {
    val t = LocalTokens.current
    var open by remember { mutableStateOf(false) }
    val act = rememberAct(changed)
    val chosen = state.chosen
    val auto = state.auto
    Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
        Box {
            Row(
                Modifier
                    .widthIn(max = 150.dp)
                    .background(if (chosen != null) t.select else t.side, RoundedCornerShape(8.dp))
                    .clickable { open = true }
                    .padding(8.dp, 5.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // The colour of the chosen scope (ADR 0060 §5).
                if (chosen?.color != null) {
                    ScopeDot(chosen.color)
                    Spacer(Modifier.width(5.dp))
                }
                Text(
                    chosen?.title ?: "Scope",
                    color = if (chosen != null) t.accent else t.muted,
                    fontWeight = if (chosen != null) FontWeight.SemiBold else FontWeight.Normal,
                    fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (auto) Text(" ·A", color = t.muted, fontSize = 11.sp)
                Text(" ▾", color = t.muted, fontSize = 11.sp)
            }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                DropdownMenuItem(
                    text = { Text(tr("Kein Scope"), fontWeight = if (chosen == null) FontWeight.SemiBold else FontWeight.Normal) },
                    onClick = { open = false; act { selectScope(null) } },
                )
                state.scopes.forEach { s ->
                    DropdownMenuItem(
                        text = {
                            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                ScopeDot(s.color)
                                Text(s.title, fontWeight = if (s.chosen) FontWeight.SemiBold else FontWeight.Normal)
                            }
                        },
                        onClick = { open = false; act { selectScope(s.id) } },
                    )
                }
                HorizontalDivider()
                DropdownMenuItem(
                    text = { Text(if (auto) tr("✓ Automatisch nach Zeitplan") else tr("Automatisch nach Zeitplan")) },
                    onClick = { open = false; act { setScopeAuto(!auto) } },
                )
                DropdownMenuItem(text = { Text(tr("Scopes bearbeiten …"), color = t.accent) }, onClick = { open = false; edit() })
            }
        }
        if (chosen != null && state.outside > 0u) {
            // Short in the title; the menu of the chip says no more.
            Text(
                tr("{outside} außerhalb", "outside" to state.outside),
                color = t.soon, fontSize = 12.sp, maxLines = 1,
                modifier = Modifier
                    .background(t.soonBg, RoundedCornerShape(8.dp))
                    .clickable { act { selectScope(null) } }
                    .padding(7.dp, 5.dp),
            )
        }
    }
}

/** A strip in the scope's colour under the header, while it is chosen. */
@Composable
fun ScopeLine(state: ScopeState) {
    scopeLine(state.chosen?.color)?.let { line ->
        Box(Modifier.fillMaxWidth().height(3.dp).background(line))
    }
}

/**
 * Settings (ADR 0035 §8, ADR 0037 §6): a list of the sections, each a
 * screen of its own. Back goes up one level.
 */
@Composable
fun SettingsScreen(tick: Int, changed: () -> Unit, start: String? = null, back: () -> Unit) {
    val t = LocalTokens.current
    // From the project tree straight to one folder or project (ADR 0085 §6).
    var section by remember { mutableStateOf(if (start != null) "places" else null) }
    BackHandler(enabled = section != null) { section = null }
    when (section) {
        "scopes" -> ScopesSection(tick, changed) { section = null }
        "places", "tags" -> PlacesScreen(section!!, tick, changed, if (section == "places") start else null) { section = null }
        "bar" -> BarSection(tick, changed) { section = null }
        "forecast" -> ForecastSection(tick, changed) { section = null }
        "calendars" -> CalendarsSection(tick, changed) { section = null }
        "reminders" -> RemindersSection { section = null }
        "properties" -> PropertiesSection(tick, changed) { section = null }
        "language" -> LanguageSection { section = null }
        "account" -> AccountSection(tick) { section = null }
        else -> LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            item { TextButton(onClick = back) { Text(tr("‹ Zurück")) } }
            items(
                listOf(
                    Triple("account", tr("Konto"), tr("Abmelden, Konto wechseln oder hinzufügen")),
                    Triple("scopes", "Scopes", tr("Ein Teil aller Ansichten, je Gerät gewählt")),
                    Triple("bar", tr("Leiste"), tr("Was unten steht, in welcher Reihenfolge, nur hier")),
                    Triple("places", tr("Projekte und Ordner"), tr("Anlegen, umbenennen, verschieben, Art und Status")),
                    Triple("tags", "Tags", tr("Anlegen, umbenennen, verschieben")),
                    Triple("forecast", "Forecast", tr("Tage, Markiert, Zurückgestelltes, Forecast-Tag")),
                    Triple("calendars", tr("Kalender"), tr("Ein/aus, Name, Farbe, jetzt abrufen")),
                    Triple("reminders", tr("Erinnerungen"), tr("An Termine erinnern, nur auf diesem Gerät")),
                    Triple("properties", "Properties", tr("Wohin ein Wert führt, je Schlüssel")),
                    Triple("language", tr("Sprache"), tr("System, Deutsch oder English, nur auf diesem Gerät")),
                ),
                key = { it.first },
            ) { (key, title, hint) ->
                Row(
                    Modifier.fillMaxWidth()
                        .background(t.side, RoundedCornerShape(8.dp))
                        .clickable { section = key }
                        .padding(14.dp, 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(title, color = t.fg, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                        Text(hint, color = t.muted, fontSize = 12.sp)
                    }
                    Text("›", color = t.muted, fontSize = 18.sp)
                }
            }
        }
    }
}

/**
 * The language of the app (ADR 0105 §2): on this phone only, like the
 * theme; a change redraws at once.
 */
@Composable
private fun LanguageSection(back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Einstellungen")) } }
        items(I18n.CHOICES, key = { it.first }) { (key, name) ->
            Row(
                Modifier.fillMaxWidth().clickable { I18n.choose(context, key) },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RadioButton(selected = I18n.choice == key, onClick = { I18n.choose(context, key) })
                Text(name, color = t.fg, fontSize = 14.sp)
            }
        }
        item { Text(tr("„System“ folgt der Sprache des Telefons und der App-Sprache in Androids Einstellungen."), color = t.muted, fontSize = 12.sp) }
    }
}

/**
 * Forecast's settings (ADR 0045 §4): the account's, the same on every
 * device; changed through the outbox, offline too.
 */
@Composable
private fun ForecastSection(tick: Int, changed: () -> Unit, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val act = rememberAct(changed)
    var settings by remember { mutableStateOf<ForecastSettings?>(null) }
    var tags by remember { mutableStateOf<List<MemberRow>>(emptyList()) }
    var days by remember { mutableStateOf("") }
    var pickTag by remember { mutableStateOf(false) }
    LaunchedEffect(tick) {
        withContext(Dispatchers.IO) {
            val samu = Store.get(context)
            settings = runCatching { samu.forecastSettings() }.getOrNull()
            // The tags as the scope settings list them, any scope will do.
            tags = runCatching {
                samu.scopes().firstOrNull()?.let { s -> samu.scopeMembers(s.id).filter { it.kind == "tag" } }
            }.getOrNull() ?: emptyList()
        }
        settings?.let { days = it.days.toString() }
    }
    val s = settings
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Einstellungen")) } }
        if (s == null || !s.known) {
            item { Text(tr("Die Einstellungen kommen mit dem nächsten Sync."), color = t.muted) }
            return@LazyColumn
        }
        item {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    days, { days = it.filter(Char::isDigit) }, singleLine = true,
                    label = { Text(tr("Tage nach heute")) }, modifier = Modifier.weight(1f),
                )
                if (days != s.days.toString() && days.isNotEmpty()) {
                    TextButton(onClick = { act { setForecastSetting("days", days) } }) { Text(tr("Sichern")) }
                }
            }
            Text(tr("Alles danach steht unter „Später“."), color = t.muted, fontSize = 12.sp)
        }
        listOf(
            Triple("flagged", tr("Abschnitt „Markiert“ nach Heute"), s.flagged),
            Triple("defer", tr("Zurückgestelltes am Tag, an dem es verfügbar wird"), s.defer),
            Triple("done", tr("Heute Erledigtes bleibt unter Heute"), s.done),
            Triple("events", tr("Termine aus den Kalendern an ihren Tagen"), s.events),
        ).forEach { (key, text, on) ->
            item(key = key) {
                Row(
                    Modifier.fillMaxWidth().clickable { act { setForecastSetting(key, if (on) "off" else "on") } },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(checked = on, onCheckedChange = { now -> act { setForecastSetting(key, if (now) "on" else "off") } })
                    Text(text, color = t.fg, fontSize = 14.sp)
                }
            }
        }
        item {
            Heading(tr("FORECAST-TAG"))
            Text(tr("Seine Aufgaben, und die der Tags darunter, stehen unter Heute."), color = t.muted, fontSize = 12.sp)
            Box {
                TextButton(onClick = { pickTag = true }) { Text(tr("Tag: ") + (s.tagPath ?: tr("keiner"))) }
                DropdownMenu(expanded = pickTag, onDismissRequest = { pickTag = false }) {
                    DropdownMenuItem(text = { Text(tr("keiner")) }, onClick = { pickTag = false; act { setForecastSetting("tag", "") } })
                    tags.forEach { tag ->
                        DropdownMenuItem(
                            text = { Text("  ".repeat(tag.depth.toInt()) + tag.title) },
                            onClick = { pickTag = false; act { setForecastSetting("tag", tag.id) } },
                        )
                    }
                }
            }
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}

/** Colours to choose from, those of the web's colour field and a few more. */
private val CalendarColors = listOf(
    "#3b82f6", "#10b981", "#f59e0b", "#ef4444", "#8b5cf6", "#ec4899", "#14b8a6", "#64748b",
)

private fun swatch(hex: String?): Color? =
    hex?.let { runCatching { Color(android.graphics.Color.parseColor(it)) }.getOrNull() }

/**
 * Calendars (ADR 0047 §8, §13): on/off, name and colour through the
 * outbox, offline too; fetching now needs the server. Adding a calendar,
 * finding them and its source stay in the web settings and the CLI: the
 * password never comes to the phone.
 */
@Composable
private fun CalendarsSection(tick: Int, changed: () -> Unit, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var error by remember { mutableStateOf<String?>(null) }
    val act = rememberAct(changed) { error = it }
    val scope = rememberCoroutineScope()
    var calendars by remember { mutableStateOf<List<CalendarRow>>(emptyList()) }
    var open by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(tick) {
        calendars = withContext(Dispatchers.IO) { runCatching { Store.get(context).calendars() }.getOrNull() } ?: emptyList()
    }
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Einstellungen")) } }
        error?.let { item { Text(it, color = t.overdue, fontSize = 13.sp) } }
        if (calendars.isEmpty()) {
            item {
                Text(
                    tr("Noch keine Kalender. Angelegt werden sie in den Einstellungen im Web oder mit samu calendar add."),
                    color = t.muted, fontSize = 13.sp,
                )
            }
        }
        items(calendars, key = { it.id }) { c ->
            Column(
                Modifier.fillMaxWidth().background(t.side, RoundedCornerShape(8.dp)).padding(4.dp, 4.dp, 12.dp, 8.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(checked = c.enabled, onCheckedChange = { on ->
                        error = null
                        act { setCalendar(c.id, "enabled", if (on) "on" else "off") }
                    })
                    Box(Modifier.size(12.dp).background(swatch(c.color) ?: t.muted, CircleShape))
                    Column(Modifier.weight(1f).clickable { open = if (open == c.id) null else c.id }.padding(start = 10.dp)) {
                        Text(
                            c.title, color = if (c.enabled) t.fg else t.muted, fontSize = 15.sp,
                            fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            listOfNotNull(
                                if (c.kind == "caldav") "CalDAV" else "ICS",
                                c.host,
                                tr("{appointments} Termine", "appointments" to c.appointments),
                            ).joinToString(" · "),
                            color = t.muted, fontSize = 12.sp,
                        )
                    }
                    Text(if (open == c.id) "⌃" else "›", color = t.muted, fontSize = 18.sp,
                        modifier = Modifier.clickable { open = if (open == c.id) null else c.id })
                }
                c.error?.let { Text(it, color = t.overdue, fontSize = 12.sp, modifier = Modifier.padding(start = 12.dp)) }
                if (open == c.id) CalendarEditor(c, busy == c.id, act) {
                    error = null
                    busy = c.id
                    scope.launch {
                        try {
                            val failed = withContext(Dispatchers.IO) { Store.get(context).refreshCalendar(c.id) }
                            error = failed?.let { "${c.title}: $it" }
                        } catch (e: Exception) {
                            error = said(e)
                        }
                        busy = null
                        changed()
                    }
                }
            }
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}

@Composable
private fun CalendarEditor(c: CalendarRow, fetching: Boolean, act: ((Samu.() -> Unit) -> Unit), fetch: () -> Unit) {
    val t = LocalTokens.current
    var title by remember(c.id, c.title) { mutableStateOf(c.title) }
    Column(Modifier.padding(start = 12.dp, top = 4.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                title, { title = it }, singleLine = true,
                label = { Text("Name") }, modifier = Modifier.weight(1f),
            )
            if (title.trim() != c.title && title.isNotBlank()) {
                TextButton(onClick = { act { setCalendar(c.id, "title", title) } }) { Text(tr("Sichern")) }
            }
        }
        Text(tr("Farbe"), color = t.muted, fontSize = 12.sp)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            CalendarColors.forEach { hex ->
                val chosen = c.color.equals(hex, ignoreCase = true)
                Box(
                    Modifier.size(28.dp)
                        .background(swatch(hex) ?: t.muted, CircleShape)
                        .border(if (chosen) 3.dp else 0.dp, if (chosen) t.fg else Color.Transparent, CircleShape)
                        .clickable { act { setCalendar(c.id, "color", hex) } },
                )
            }
        }
        val custom = c.color != null && CalendarColors.none { it.equals(c.color, ignoreCase = true) }
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (custom) Text(tr("eigene: {color}", "color" to c.color), color = t.muted, fontSize = 12.sp, modifier = Modifier.weight(1f))
            else Spacer(Modifier.weight(1f))
            if (c.color != null) TextButton(onClick = { act { setCalendar(c.id, "color", "") } }) { Text(tr("Keine Farbe")) }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                c.fetchedAt?.let { at ->
                    runCatching {
                        tr("Zuletzt geändert: ") + java.time.Instant.parse(at).atZone(java.time.ZoneId.systemDefault())
                            .format(java.time.format.DateTimeFormatter.ofPattern("dd.MM. HH:mm"))
                    }.getOrNull()
                } ?: tr("Noch nicht abgerufen"),
                color = t.muted, fontSize = 12.sp, modifier = Modifier.weight(1f),
            )
            OutlinedButton(onClick = fetch, enabled = !fetching) { Text(if (fetching) tr("Hole …") else tr("Jetzt abrufen")) }
        }
    }
}

/**
 * Reminders of appointments (ADR 0101 §6): a switch on this phone only,
 * off unless set, not synced. A phone whose calendar app syncs the same
 * calendars (DAVx5) reminds of their VALARMs already.
 */
@Composable
private fun RemindersSection(back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var on by remember { mutableStateOf<Boolean?>(null) }
    LaunchedEffect(Unit) {
        on = withContext(Dispatchers.IO) { runCatching { Store.get(context).appointmentReminders() }.getOrNull() } ?: false
    }
    Column(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        TextButton(onClick = back) { Text(tr("‹ Einstellungen")) }
        Row(
            Modifier.fillMaxWidth().background(t.side, RoundedCornerShape(8.dp)).padding(14.dp, 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(tr("Erinnerungen an Termine"), color = t.fg, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                Text(tr("Wann, steht im Termin selbst (VALARM)."), color = t.muted, fontSize = 12.sp)
            }
            Switch(checked = on == true, enabled = on != null, onCheckedChange = { wanted ->
                on = wanted
                scope.launch {
                    withContext(Dispatchers.IO) {
                        runCatching { Store.get(context).setAppointmentReminders(wanted, System.currentTimeMillis()) }
                    }
                    Reminders.arm(context)
                }
            })
        }
        Text(
            tr("Gleicht dieses Handy die Kalender schon über DAVx5 ab, erinnert die Kalender-App selbst daran – dann hier aus lassen, sonst kommt jede Erinnerung zweimal."),
            color = t.muted, fontSize = 13.sp,
        )
        Text(tr("Erinnerungen an Aufgaben kommen immer."), color = t.muted, fontSize = 13.sp)
    }
}

/**
 * Scopes (ADR 0035 §8): make one, name it, choose its mode, tick its
 * members, use it here, delete it. Every edit goes through the outbox like
 * any other change.
 */
@Composable
private fun ScopesSection(tick: Int, changed: () -> Unit, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var scopes by remember { mutableStateOf<List<ScopeRow>>(emptyList()) }
    var editing by remember { mutableStateOf<String?>(null) }
    var naming by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(tick) {
        scopes = withContext(Dispatchers.IO) { runCatching { Store.get(context).scopes() }.getOrDefault(emptyList()) }
        if (editing != null && scopes.none { it.id == editing }) editing = null
    }
    val act = rememberAct(changed) { error = it }
    val current = scopes.firstOrNull { it.id == editing }
    BackHandler(enabled = current != null) { editing = null }
    if (current != null) {
        ScopeEditor(current, tick, act, error) { editing = null }
        return
    }
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Einstellungen")) } }
        item {
            Text(tr("SCOPES"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.7.sp)
            Text(
                tr("Ein Scope zeigt in allen Ansichten nur einen Teil, etwa nur Privat oder alles außer IT. Gewählt wird je Gerät."),
                color = t.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 4.dp),
            )
        }
        items(scopes, key = { it.id }) { s ->
            Row(
                Modifier.fillMaxWidth()
                    .background(if (s.chosen) t.select else t.side, RoundedCornerShape(8.dp))
                    .clickable { error = null; editing = s.id }
                    .padding(14.dp, 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(s.title, color = t.fg, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                    Text(if (s.only) tr("nur diese") else tr("alle außer diesen"), color = t.muted, fontSize = 12.sp)
                }
                if (s.chosen) Text(tr("hier gewählt"), color = t.accent, fontSize = 12.sp)
            }
        }
        item {
            OutlinedButton(onClick = { naming = true }, modifier = Modifier.fillMaxWidth()) { Text(tr("+ Neuer Scope")) }
        }
        error?.let { item { Text(it, color = t.overdue, fontSize = 13.sp) } }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
    if (naming) {
        var name by remember { mutableStateOf("") }
        val scope = rememberCoroutineScope()
        AlertDialog(
            onDismissRequest = { naming = false },
            title = { Text(tr("Neuer Scope")) },
            text = { OutlinedTextField(name, { name = it }, singleLine = true, placeholder = { Text(tr("z. B. Ohne IT")) }) },
            confirmButton = {
                TextButton(enabled = name.isNotBlank(), onClick = {
                    naming = false
                    scope.launch {
                        val id = withContext(Dispatchers.IO) {
                            runCatching { Store.get(context).createScope(name, false) }.getOrNull()
                        }
                        changed()
                        editing = id
                    }
                }) { Text(tr("Anlegen")) }
            },
            dismissButton = { TextButton(onClick = { naming = false }) { Text(tr("Abbrechen")) } },
        )
    }
}

@Composable
private fun ScopeEditor(
    scope: ScopeRow,
    tick: Int,
    act: (Samu.() -> Unit) -> Unit,
    error: String?,
    back: () -> Unit,
) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var members by remember { mutableStateOf<List<MemberRow>>(emptyList()) }
    var name by remember(scope.id) { mutableStateOf(scope.title) }
    var deleting by remember(scope.id) { mutableStateOf(false) }
    // The filter as text in the search language (ADR 0043 §2): what is
    // stored, and what is being typed.
    var stored by remember(scope.id) { mutableStateOf("") }
    var filter by remember(scope.id) { mutableStateOf("") }
    // What a capture here gets when its line names none (ADR 0043 §6).
    var capture by remember(scope.id) { mutableStateOf(ScopeCaptureRow(null, emptyList())) }
    var pickProject by remember(scope.id) { mutableStateOf(false) }
    // The schedule as text (ADR 0043 §7): stored, and being typed.
    var storedWindows by remember(scope.id) { mutableStateOf("") }
    var windows by remember(scope.id) { mutableStateOf("") }
    // How the task page looks in this scope (ADR 0072); none = all of it.
    var layout by remember(scope.id) { mutableStateOf<InspectorLayoutRow?>(null) }
    LaunchedEffect(tick, scope.id) {
        layout = withContext(Dispatchers.IO) { runCatching { Store.get(context).scopeLayout(scope.id) }.getOrNull() }
        members = withContext(Dispatchers.IO) { runCatching { Store.get(context).scopeMembers(scope.id) }.getOrDefault(emptyList()) }
        val now = withContext(Dispatchers.IO) { runCatching { Store.get(context).scopeFilter(scope.id) }.getOrDefault("") }
        if (filter == stored) filter = now
        stored = now
        val nowWindows = withContext(Dispatchers.IO) { runCatching { Store.get(context).scopeSchedule(scope.id) }.getOrDefault("") }
        if (windows == storedWindows) windows = nowWindows
        storedWindows = nowWindows
        capture = withContext(Dispatchers.IO) {
            runCatching { Store.get(context).scopeCapture(scope.id) }.getOrDefault(ScopeCaptureRow(null, emptyList()))
        }
    }
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Scopes")) } }
        item {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(name, { name = it }, singleLine = true, label = { Text("Name") }, modifier = Modifier.weight(1f))
                if (name.trim() != scope.title && name.isNotBlank()) {
                    TextButton(onClick = { act { renameScope(scope.id, name) } }) { Text(tr("Sichern")) }
                }
            }
        }
        item {
            Heading(tr("FARBE"))
            ScopeColorPicker(scope.color) { c -> act { setScopeColor(scope.id, c) } }
        }
        item { LayoutEditor(layout) { next -> act { setScopeLayout(scope.id, next) } } }
        item {
            Text(tr("DIE ANSICHTEN ZEIGEN"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 8.dp))
            listOf(true to tr("nur, was die Häkchen erfassen"), false to tr("alles außer dem, was die Häkchen erfassen")).forEach { (only, text) ->
                Row(
                    Modifier.fillMaxWidth().clickable { act { setScopeMode(scope.id, only) } },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = scope.only == only, onClick = { act { setScopeMode(scope.id, only) } })
                    Text(text, color = t.fg, fontSize = 14.sp)
                }
            }
            Text(
                tr("Ein Ordner gilt mit allem darin, ein Tag mit allen Tags darunter und für jede Aufgabe eines Projekts, das ihn trägt. Die Inbox zählt nur über ihr eigenes Häkchen. Ohne jedes Häkchen schränken die Mitglieder nicht ein; dann zählt nur der Filter."),
                color = t.muted, fontSize = 12.sp,
            )
        }
        item {
            Heading(tr("FILTER"))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    filter, { filter = it }, singleLine = true,
                    placeholder = { Text(tr("fällig:7t ist:markiert -@Warten")) },
                    modifier = Modifier.weight(1f),
                )
                if (filter.trim() != stored) {
                    TextButton(onClick = { act { setScopeFilter(scope.id, filter) } }) { Text(tr("Sichern")) }
                }
            }
            Text(
                tr("Dazu muss eine Aufgabe auch das erfüllen. Wie in der Suche: #Projekt, @Tag, ordner:, fällig:/geplant:/start: mit heute, morgen, woche, 7t, ist:markiert, ~<15m; - davor schließt aus."),
                color = t.muted, fontSize = 12.sp,
            )
        }
        item {
            Heading(tr("ZEITPLAN"))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    windows, { windows = it }, singleLine = true,
                    placeholder = { Text("mo-fr 08:00-17:00; sa 22:00-02:00") },
                    modifier = Modifier.weight(1f),
                )
                if (windows.trim() != storedWindows) {
                    TextButton(onClick = { act { setScopeSchedule(scope.id, windows) } }) { Text(tr("Sichern")) }
                }
            }
            Text(
                tr("Wann ein Gerät, das „automatisch“ folgt, diesen Scope wählt. Endet ein Fenster nicht nach seinem Beginn, endet es am Folgetag. Eine Wahl von Hand oder von Tasker gilt bis zum nächsten Fensterrand."),
                color = t.muted, fontSize = 12.sp,
            )
        }
        item {
            Heading(tr("NEUE AUFGABEN BEKOMMEN"))
            Text(
                tr("…wenn die Zeile beim Erfassen kein #Projekt bzw. keinen @Tag nennt."),
                color = t.muted, fontSize = 12.sp,
            )
            val projects = members.filter { it.kind == "project" }
            Box {
                TextButton(onClick = { pickProject = true }) {
                    Text(tr("Projekt: ") + (projects.firstOrNull { it.id == capture.project }?.title ?: "Inbox"))
                }
                DropdownMenu(expanded = pickProject, onDismissRequest = { pickProject = false }) {
                    DropdownMenuItem(text = { Text("Inbox") }, onClick = {
                        pickProject = false
                        act { setScopeCapture(scope.id, null, capture.tags) }
                    })
                    projects.forEach { p ->
                        DropdownMenuItem(text = { Text(p.title) }, onClick = {
                            pickProject = false
                            act { setScopeCapture(scope.id, p.id, capture.tags) }
                        })
                    }
                }
            }
            members.filter { it.kind == "tag" }.forEach { tag ->
                val on = tag.id in capture.tags
                Row(
                    Modifier.fillMaxWidth().clickable {
                        act { setScopeCapture(scope.id, capture.project, if (on) capture.tags - tag.id else capture.tags + tag.id) }
                    }.padding(start = (tag.depth.toInt() * 20).dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(checked = on, onCheckedChange = { now ->
                        act { setScopeCapture(scope.id, capture.project, if (now) capture.tags + tag.id else capture.tags - tag.id) }
                    })
                    Text("@" + tag.title, color = t.fg, fontSize = 14.sp)
                }
            }
        }
        val folders = members.filter { it.kind != "tag" }
        val tags = members.filter { it.kind == "tag" }
        item { Heading(tr("ORDNER UND PROJEKTE")) }
        // The inbox, a member of its own (ADR 0043 §1), above the folders.
        item(key = "inbox") {
            MemberLine(MemberRow("inbox", "Inbox", "inbox", 0u, scope.inbox, false, null)) { on -> act { setScopeInbox(scope.id, on) } }
        }
        items(folders, key = { "f" + it.id }) { m -> MemberLine(m) { on -> act { setScopeMember(scope.id, m.id, on) } } }
        if (tags.isNotEmpty()) item { Heading(tr("TAGS")) }
        items(tags, key = { "t" + it.id }) { m -> MemberLine(m) { on -> act { setScopeMember(scope.id, m.id, on) } } }
        error?.let { item { Text(it, color = t.overdue, fontSize = 13.sp) } }
        item {
            Row(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (scope.chosen) {
                    OutlinedButton(onClick = { act { selectScope(null) } }) { Text(tr("Hier aufheben")) }
                } else {
                    Button(onClick = { act { selectScope(scope.id) } }) { Text(tr("Hier verwenden")) }
                }
                Spacer(Modifier.weight(1f))
                if (deleting) {
                    Button(
                        onClick = { act { deleteScope(scope.id) }; back() },
                        colors = ButtonDefaults.buttonColors(containerColor = t.overdue),
                    ) { Text(tr("Wirklich löschen?")) }
                } else {
                    OutlinedButton(onClick = { deleting = true }) { Text(tr("Löschen")) }
                }
            }
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}

/**
 * "Gestalt" (ADR 0072 §5): a chip per part of the task page, the note,
 * the preset "Habit" and "Alles zeigen". Written as a whole.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun LayoutEditor(layout: InspectorLayoutRow?, write: (InspectorLayoutRow?) -> Unit) {
    val t = LocalTokens.current
    val shown = layout?.shown ?: Words.PARTS.map { it.first }
    val rendered = layout?.noteRendered == true
    Heading(tr("GESTALT"))
    Text(tr("Was die Aufgabe zeigt; der Rest steht eingeklappt unter „Mehr“."), color = t.muted, fontSize = 12.sp)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Words.PARTS.forEach { (key, label) ->
            val on = key in shown
            FilterChip(
                selected = on,
                onClick = { write(InspectorLayoutRow(if (on) shown - key else shown + key, rendered)) },
                label = { Text(label, fontSize = 13.sp, color = t.fg) },
            )
        }
    }
    Row(
        Modifier.fillMaxWidth().clickable { write(InspectorLayoutRow(shown, !rendered)) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = rendered, onCheckedChange = { on -> write(InspectorLayoutRow(shown, on)) })
        Text(tr("Notiz nur gerendert"), color = t.fg, fontSize = 14.sp)
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton(onClick = { write(InspectorLayoutRow(listOf("note", "repeat", "planned", "reminders"), true)) }) { Text(tr("Vorgabe Habit")) }
        OutlinedButton(onClick = { write(null) }, enabled = layout != null) { Text(tr("Alles zeigen")) }
    }
    Text(
        tr("Ort, Titel und Erledigen stehen immer da. Eingeklapptes mit Wert nennt die Zeile „Mehr“ beim Namen. Aufklappen gilt nur auf diesem Gerät."),
        color = t.muted, fontSize = 12.sp,
    )
}

@Composable
internal fun Heading(text: String) {
    Text(
        text, color = LocalTokens.current.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold,
        letterSpacing = 0.7.sp, modifier = Modifier.padding(top = 12.dp),
    )
}

@Composable
private fun MemberLine(m: MemberRow, toggle: (Boolean) -> Unit) {
    val t = LocalTokens.current
    Row(
        Modifier.fillMaxWidth().clickable { toggle(!m.member) }.padding(start = (m.depth.toInt() * 20).dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = m.member, onCheckedChange = { toggle(it) })
        Text(
            m.title + (m.ownScopeTitle?.let { " ◌ $it" } ?: if (m.scopeOnly) " ◌" else ""),
            color = if (m.kind == "folder") t.muted else t.fg,
            fontWeight = if (m.kind == "folder") FontWeight.SemiBold else FontWeight.Normal,
            fontSize = 14.sp,
        )
    }
}
