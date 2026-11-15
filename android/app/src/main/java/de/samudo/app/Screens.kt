package de.samudo.app

import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.Anchor
import de.samudo.app.mobile.AppointmentRow
import de.samudo.app.mobile.BlockerRow
import de.samudo.app.mobile.CapturePreview
import de.samudo.app.mobile.Offer
import de.samudo.app.mobile.Span
import de.samudo.app.mobile.Day
import de.samudo.app.mobile.MobileException
import de.samudo.app.mobile.ClockRow
import de.samudo.app.mobile.ReminderRow
import de.samudo.app.mobile.RunningClock
import de.samudo.app.mobile.Samu
import de.samudo.app.mobile.Section
import de.samudo.app.mobile.Status
import de.samudo.app.mobile.TagChip
import de.samudo.app.mobile.TaskDetail
import de.samudo.app.mobile.TaskEdit
import de.samudo.app.mobile.TaskRow
import de.samudo.app.mobile.TaskLayout
import de.samudo.app.mobile.TemplateRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** The text of an error, as the screen shows it. */
internal fun said(e: Throwable): String = when (e) {
    is MobileException.SignedOut -> e.reason
    is MobileException.Offline -> tr("Server nicht erreichbar: {reason}", "reason" to e.reason)
    is MobileException.Refused -> e.reason
    is MobileException.Input -> e.reason
    is MobileException.Store -> e.reason
    else -> e.toString()
}

/** Signing in with server, account, password and device (ADR 0028 §8). */
@Composable
fun LoginScreen(
    status: Status,
    // The other accounts on the phone, to switch to instead (ADR 0107 §5).
    others: List<AccountLine> = emptyList(),
    // Back to the account left, while one is being added (§4).
    cancel: (() -> Unit)? = null,
    onSignedIn: () -> Unit,
) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var server by remember { mutableStateOf(status.server ?: "") }
    var account by remember { mutableStateOf(status.account ?: "") }
    var password by remember { mutableStateOf("") }
    var device by remember { mutableStateOf(status.device ?: Build.MODEL.lowercase().replace(' ', '-')) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    Column(
        Modifier.fillMaxSize().background(t.bg).imePadding().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Spacer(Modifier.heightIn(min = 48.dp))
        // The lockup of ADR 0050; drawable-night holds the one for dark ground.
        androidx.compose.foundation.Image(
            androidx.compose.ui.res.painterResource(R.drawable.samudo_lockup),
            contentDescription = "samudō",
            modifier = Modifier.heightIn(max = 64.dp),
        )
        if (status.outbox > 0u) {
            Text(tr("{outbox} Änderungen warten im Ausgang; sie gehen nach dem Anmelden raus.", "outbox" to status.outbox), color = t.wait, fontSize = 13.sp)
        }
        OutlinedTextField(server, { server = it }, label = { Text("Server") }, singleLine = true,
            placeholder = { Text("https://samu.example.org") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri), modifier = Modifier.fillMaxWidth())
        OutlinedTextField(account, { account = it }, label = { Text(tr("Konto")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(password, { password = it }, label = { Text(tr("Passwort")) }, singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), modifier = Modifier.fillMaxWidth())
        OutlinedTextField(device, { device = it }, label = { Text(tr("Gerät")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
        error?.let { Text(it, color = t.overdue) }
        Button(
            enabled = !busy && account.isNotBlank() && password.isNotEmpty(),
            onClick = {
                busy = true
                error = null
                scope.launch {
                    try {
                        withContext(Dispatchers.IO) { Store.get(context).login(server, account, password, device) }
                        onSignedIn()
                    } catch (e: Exception) {
                        error = said(e)
                    } finally {
                        busy = false
                    }
                }
            },
            modifier = Modifier.fillMaxWidth(),
        ) { Text(if (busy) "…" else tr("Anmelden")) }
        // ADR 0031: the code from "Handy koppeln" in the web UI brings
        // server, account and device name; no password typed here.
        Text(tr("oder"), color = t.muted, fontSize = 13.sp, modifier = Modifier.align(Alignment.CenterHorizontally))
        OutlinedButton(
            enabled = !busy,
            onClick = {
                error = null
                scanPairingCode(context, onCode = { uri ->
                    busy = true
                    scope.launch {
                        try {
                            withContext(Dispatchers.IO) { Store.get(context).pair(uri) }
                            onSignedIn()
                        } catch (e: Exception) {
                            error = said(e)
                        } finally {
                            busy = false
                        }
                    }
                }, onError = { error = it })
            },
            modifier = Modifier.fillMaxWidth(),
        ) { Text(tr("QR-Code scannen")) }
        Text(tr("Den Code zeigt die Web-UI im Kontomenü unter „Handy koppeln“."), color = t.muted, fontSize = 12.sp)
        others.forEach { other ->
            TextButton(onClick = { Accounts.switch(context, other.slot) }, modifier = Modifier.fillMaxWidth()) {
                Text(tr("⇄ {label}", "label" to other.label))
            }
        }
        cancel?.let { TextButton(onClick = it, modifier = Modifier.fillMaxWidth()) { Text(tr("Abbrechen")) } }
    }
}

/**
 * Loads from samu-mobile off the main thread, again on each tick and when
 * `key` (what the list shows: tab, project, tag, search) changes.
 */
@Composable
internal fun <T> loaded(tick: Int, key: Any?, load: (Samu) -> T): Pair<T?, String?> {
    val context = LocalContext.current
    var value by remember { mutableStateOf<T?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(tick, key) {
        try {
            value = withContext(Dispatchers.IO) { load(Store.get(context)) }
            error = null
        } catch (e: kotlinx.coroutines.CancellationException) {
            // A load given up for a newer one, or a list that left the
            // screen: no error to show.
            throw e
        } catch (e: Exception) {
            error = said(e)
        }
    }
    return value to error
}

/** Sections of rows, loaded from samu-mobile off the main thread. */
@Composable
fun TaskList(
    tick: Int,
    key: Any?,
    load: (Samu) -> List<Section>,
    changed: () -> Unit,
    empty: String = tr("Nichts hier."),
    open: (String) -> Unit,
    // Above the first row, scrolling away with the list (the tag chips, ADR 0102 §6).
    head: (@Composable () -> Unit)? = null,
) {
    val t = LocalTokens.current
    val (sections, error) = loaded(tick, key, load)
    error?.let { Text(it, color = t.overdue, modifier = Modifier.padding(16.dp)) }
    val list = sections ?: return
    LaunchedEffect(list) { Picks.refresh(list.flatMap { it.rows }) }
    // A list, even an empty one, so that pulling down refreshes it (ADR 0102 §6).
    LazyColumn(Modifier.fillMaxSize()) {
        // At least 1 dp: a head of zero height is not the first visible item,
        // so chips loaded after the rows would land above the top.
        head?.let { item(key = "list-head") { Box(Modifier.heightIn(min = 1.dp)) { it() } } }
        if (list.isEmpty()) item(key = "empty") { Text(empty, color = t.muted, modifier = Modifier.padding(16.dp)) }
        list.forEach { section ->
            if (section.title != null || list.size > 1) {
                item(key = "head-" + (section.projectId ?: "inbox")) {
                    // The folder with its whole path next to the project (ADR 0026 §6).
                    Row(Modifier.padding(16.dp, 18.dp, 16.dp, 6.dp), verticalAlignment = Alignment.Bottom) {
                        Text(
                            (section.title ?: "Inbox").uppercase(),
                            fontSize = 12.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.7.sp, color = t.muted,
                        )
                        section.folder?.let { Text("  $it", fontSize = 12.sp, color = t.muted, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                    }
                }
            }
            items(section.rows, key = { it.id }) { row -> SwipeRow(row, changed) { open(row.id) } }
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}

/**
 * Forecast as samu-core computes it (ADR 0045): a strip of the sections
 * with their counts, then Überfällig, Heute, Markiert, the days, Später;
 * each row says why it stands there.
 */
@Composable
fun ForecastList(tick: Int, tag: String?, changed: () -> Unit, open: (String) -> Unit, head: (@Composable () -> Unit)? = null) {
    val t = LocalTokens.current
    val context = LocalContext.current
    val (days, error) = loaded<List<Day>>(tick, tag) { it.forecast(tag) }
    error?.let { Text(it, color = t.overdue, modifier = Modifier.padding(16.dp)) }
    val list = days ?: return
    LaunchedEffect(list) { Picks.refresh(list.flatMap { it.rows }) }
    val keyOf = { day: Day -> day.kind + (day.date ?: "") }
    val state = androidx.compose.foundation.lazy.rememberLazyListState()
    val scope = rememberCoroutineScope()
    LazyColumn(Modifier.fillMaxSize(), state = state) {
        // At least 1 dp: a head of zero height is not the first visible item,
        // so chips loaded after the rows would land above the top.
        head?.let { item(key = "list-head") { Box(Modifier.heightIn(min = 1.dp)) { it() } } }
        item(key = "strip") {
            androidx.compose.foundation.lazy.LazyRow(
                contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                modifier = Modifier.padding(vertical = 6.dp),
            ) {
                items(list, key = { keyOf(it) }) { day ->
                    Column(
                        Modifier
                            .background(if (day.today) t.select else t.side, RoundedCornerShape(8.dp))
                            .clickable {
                                // The strip is item 0, after the head; each section is a heading and its rows.
                                var index = if (head != null) 2 else 1
                                for (d in list) {
                                    if (d === day) break
                                    index += 1 + d.appointments.size + d.rows.size +
                                        (if (d.rows.isEmpty() && d.appointments.isEmpty() && d.today) 1 else 0)
                                }
                                scope.launch { state.animateScrollToItem(index) }
                            }
                            .padding(10.dp, 6.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(Words.forecastDay(day), color = t.muted, fontSize = 12.sp)
                        Text(
                            "${day.rows.size}", fontWeight = FontWeight.SemiBold, fontSize = 14.sp,
                            color = if (day.kind == "overdue") t.overdue else t.fg,
                        )
                        // A dot for a day with appointments, not their number
                        // (ADR 0047 §10).
                        Box(
                            Modifier.padding(top = 2.dp).size(5.dp).background(
                                if (day.appointments.isEmpty()) Color.Transparent else t.accent,
                                CircleShape,
                            ),
                        )
                    }
                }
            }
        }
        list.forEach { day ->
            item(key = "head-" + keyOf(day)) {
                // A long press on a day's heading starts a new appointment
                // over that day in the calendar app (ADR 0049 §4).
                val date = day.date.takeIf { day.kind == "day" }
                Text(
                    Words.forecastDay(day).uppercase(),
                    fontSize = 12.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.7.sp,
                    color = if (day.kind == "overdue") t.overdue else t.muted,
                    modifier = Modifier.fillMaxWidth()
                        .then(
                            if (date != null) Modifier.combinedClickable(onClick = {}, onLongClick = { CalendarApp.insertDay(context, date) })
                            else Modifier,
                        )
                        .padding(16.dp, 18.dp, 16.dp, 6.dp),
                )
            }
            items(day.appointments, key = { "appt-" + keyOf(day) + it.id }) { a -> AppointmentLine(a) }
            if (day.rows.isEmpty() && day.appointments.isEmpty() && day.today) {
                item(key = "empty-" + keyOf(day)) {
                    Text(tr("Nichts für heute."), color = t.muted, modifier = Modifier.padding(16.dp, 0.dp))
                }
            }
            items(day.rows, key = { keyOf(day) + it.id }) { row -> SwipeRow(row, changed, Words.why(row, day), place = row.place ?: "Inbox") { open(row.id) } }
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}

/**
 * An appointment of a calendar in Forecast (ADR 0047 §10): the colour of
 * its calendar, its time, title and place. A tap opens its sheet, with the
 * way into the calendar app and to the original (ADR 0049 §2).
 */
@Composable
fun AppointmentLine(a: AppointmentRow) {
    val t = LocalTokens.current
    var shown by remember { mutableStateOf(false) }
    val color = a.color?.let { runCatching { Color(android.graphics.Color.parseColor(it)) }.getOrNull() } ?: t.muted
    val time = if (a.allDay) tr("ganztägig") else listOfNotNull(a.start, a.end).joinToString("–")
    Row(
        Modifier.fillMaxWidth()
            .clickable { shown = true }
            .padding(16.dp, 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(Modifier.size(10.dp).background(color, CircleShape))
        Text(time, color = t.muted, fontSize = 13.sp, modifier = Modifier.width(92.dp))
        Column(Modifier.weight(1f)) {
            Text(
                a.title + if (a.tentative) tr(" (vorläufig)") else "",
                color = t.fg, fontSize = 15.sp,
            )
            a.location?.let { Text(it, color = t.muted, fontSize = 12.sp) }
        }
    }
    if (shown) AppointmentSheet(a) { shown = false }
}

/**
 * Tags as chips above a list; one at a time, a second tap takes it off
 * (ADR 0028 §9). A tag gone from the data takes itself off.
 */
@Composable
fun TagChips(tick: Int, selected: String?, onSelect: (String?) -> Unit) {
    val t = LocalTokens.current
    val (chips, _) = loaded<List<TagChip>>(tick, null) { it.tags() }
    val list = chips ?: return
    LaunchedEffect(list, selected) {
        if (selected != null && list.none { it.id == selected }) onSelect(null)
    }
    if (list.isEmpty()) return
    LazyRow(
        Modifier.fillMaxWidth().padding(bottom = 4.dp),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(list, key = { it.id }) { chip ->
            FilterChip(
                selected = chip.id == selected,
                onClick = { onSelect(if (chip.id == selected) null else chip.id) },
                label = { Text("${chip.path}  ${chip.count}", fontSize = 13.sp, color = t.fg) },
            )
        }
    }
}

/**
 * Swipe to the right completes (ADR 0020 §1); on a habit it means "today
 * done" and never completes it, on one to avoid it does nothing (ADR 0092
 * §2).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SwipeRow(row: TaskRow, changed: () -> Unit, why: String? = null, place: String? = null, open: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val habit = row.habit
    var menu by remember { mutableStateOf(false) }
    val act: (Samu.() -> Unit) -> Unit = { action ->
        scope.launch {
            withContext(Dispatchers.IO) { runCatching { Store.get(context).action() } }
            changed()
        }
    }
    // Completing and flagging offer „Rückgängig“ (ADR 0104 §1).
    val undoable: (String, Samu.() -> List<Long>, Samu.() -> Unit) -> Unit = { text, action, back ->
        scope.launch {
            runCatching { Undo.run(context, text, action, back) }
            changed()
        }
    }
    val complete: () -> Unit = {
        when {
            habit == null && row.done -> act { reopen(row.id) }
            habit == null -> undoable(tr("Erledigt"), { complete(row.id) }, { reopen(row.id) })
            habit.kind != "avoid" -> act { enterHabit(row.id, "done", null, null) }
        }
    }
    val flag: () -> Unit = {
        val on = !row.flagged
        undoable(if (on) tr("Markiert") else tr("Markierung weg"), { setFlag(row.id, on) }, { setFlag(row.id, !on) })
    }
    // Right completes (ADR 0020), left flags or unflags (ADR 0102 §7).
    val state = rememberSwipeToDismissBoxState(
        confirmValueChange = { value ->
            when (value) {
                SwipeToDismissBoxValue.StartToEnd -> complete()
                SwipeToDismissBoxValue.EndToStart -> flag()
                else -> {}
            }
            false
        },
    )
    SwipeToDismissBox(
        state = state,
        enableDismissFromEndToStart = habit == null,
        enableDismissFromStartToEnd = habit?.kind != "avoid",
        backgroundContent = {
            if (state.dismissDirection == SwipeToDismissBoxValue.EndToStart) {
                Box(Modifier.fillMaxSize().background(t.flag).padding(end = 20.dp), contentAlignment = Alignment.CenterEnd) {
                    Text(if (row.flagged) tr("Markierung weg") else tr("⚑  Markieren"), color = Color.White, fontWeight = FontWeight.SemiBold)
                }
            } else {
                Box(Modifier.fillMaxSize().background(t.ok).padding(start = 20.dp), contentAlignment = Alignment.CenterStart) {
                    val text = when {
                        habit != null -> if (Words.habitCounts(habit)) tr("+1  heute") else tr("✓  Heute geschafft")
                        row.done -> tr("Wieder öffnen")
                        else -> tr("✓  Erledigt")
                    }
                    Text(text, color = Color.White, fontWeight = FontWeight.SemiBold)
                }
            }
        },
    ) {
        // Long press opens the task's menu (ADR 0102 §7), a habit's its
        // own (ADR 0092 §2). While picking, a tap or a long press takes a
        // task in or out instead (ADR 0062 §5).
        Box {
            TaskLine(
                row, complete,
                open = { if (Picks.active) Picks.toggle(row) else open() },
                why = why,
                place = place,
                picked = Picks.has(row.id),
            ) { if (Picks.active) Picks.toggle(row) else menu = true }
            if (habit != null) HabitMenu(row.id, habit, menu, { menu = false }, act)
            else if (menu) TaskMenu(row, act, flag) { menu = false }
        }
    }
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
private fun TaskLine(row: TaskRow, toggle: () -> Unit, open: () -> Unit, why: String? = null, place: String? = null, picked: Boolean = false, longPress: () -> Unit) {
    val t = LocalTokens.current
    val haptic = androidx.compose.ui.platform.LocalHapticFeedback.current
    val dim = !row.available && !row.done
    Row(
        Modifier.fillMaxWidth().background(if (picked) t.select else t.pane)
            .combinedClickable(onClick = open, onLongClick = {
                haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress)
                longPress()
            })
            .heightIn(min = 56.dp).padding(start = (16 + 28 * row.depth.toInt()).dp, end = 16.dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val ring = when {
            row.done -> t.accent
            row.dueState == de.samudo.app.mobile.Due.OVERDUE -> t.overdue
            else -> t.muted
        }
        // The priority as a bar beside the circle (ADR 0089); empty keeps the place.
        PriorityBar(row.priority, faded = dim || row.done)
        Spacer(Modifier.width(5.dp))
        val habit = row.habit
        if (habit != null && !row.done && !row.dropped) {
            HabitMark(habit, toggle)
        } else Box(
            Modifier.size(24.dp).border(2.dp, ring, CircleShape)
                .background(if (row.done) t.accent else Color.Transparent, CircleShape)
                .clickable(onClick = toggle),
            contentAlignment = Alignment.Center,
        ) { if (row.done) Text("✓", color = t.onAccent, fontSize = 14.sp) }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(
                // 🚧 when an open task waits for it; the note mark (ADR 0053 §5).
                (if (row.blocks) "🚧 " else "") + row.title + (if (row.hasNote) "  ✎" else ""),
                color = if (dim || row.done) t.muted else t.fg,
                fontWeight = if (row.group) FontWeight.SemiBold else FontWeight.Normal,
                textDecoration = if (row.done) TextDecoration.LineThrough else null,
                fontSize = 15.sp,
            )
            val detail = listOfNotNull(why, Words.detail(row, Details.on.value).ifEmpty { null }).joinToString(" · ")
            if (detail.isNotEmpty()) {
                Text(
                    detail, fontSize = 13.sp,
                    color = when (row.dueState) {
                        de.samudo.app.mobile.Due.OVERDUE -> t.overdue
                        de.samudo.app.mobile.Due.SOON -> t.soon
                        else -> t.muted
                    },
                )
            }
            place?.let { Text(it, fontSize = 12.sp, color = t.muted, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
        row.habit?.takeIf { !row.done && !row.dropped }?.let { HabitFigures(it) }
        if (row.flagged) Text("⚑", color = t.flag, fontSize = 16.sp)
    }
    Box(Modifier.fillMaxWidth().heightIn(max = 1.dp).background(t.rowline)) { Spacer(Modifier.heightIn(min = 1.dp)) }
}

/** The id under the title, a tap copies it (01a1c271). */
@Composable
private fun IdLine(id: String, createdMs: Long) {
    val context = LocalContext.current
    val t = LocalTokens.current
    Column {
        Row(
            Modifier.clickable {
                val clipboard = context.getSystemService(android.content.ClipboardManager::class.java)
                clipboard?.setPrimaryClip(android.content.ClipData.newPlainText("samu id", id))
                // Android 13 and later confirm a copy themselves.
                if (android.os.Build.VERSION.SDK_INT < 33) {
                    android.widget.Toast.makeText(context, tr("ID kopiert"), android.widget.Toast.LENGTH_SHORT).show()
                }
            },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                id,
                color = t.muted,
                fontSize = 11.sp,
                fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                maxLines = 1,
                modifier = Modifier.weight(1f, fill = false),
            )
            Text(tr("  ⧉ kopieren"), color = t.muted, fontSize = 12.sp)
        }
        // When it was captured (a1f43aed); `erstellt:` in the search filters by it.
        Text(tr("erfasst ") + Words.instant(createdMs), color = t.muted, fontSize = 12.sp)
    }
}

/**
 * One task: what it is, every reason, and everything to edit (ADR 0028 §8,
 * ADR 0040 §2). Each field saves on its own through the outbox.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TaskScreen(id: String, tick: Int, changed: () -> Unit, open: (String) -> Unit = {}, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var detail by remember { mutableStateOf<TaskDetail?>(null) }
    var edit by remember { mutableStateOf<TaskEdit?>(null) }
    var layout by remember { mutableStateOf<TaskLayout?>(null) }
    var moving by remember { mutableStateOf(false) }
    var blocking by remember { mutableStateOf(false) }
    LaunchedEffect(tick, id) {
        withContext(Dispatchers.IO) {
            val samu = Store.get(context)
            detail = runCatching { samu.task(id) }.getOrNull()
            edit = runCatching { samu.taskEdit(id) }.getOrNull()
            layout = runCatching { samu.taskLayout(id) }.getOrNull()
        }
    }
    // The scope's layout (ADR 0072): without one, or unfolded, all parts.
    val show: (String) -> Boolean = { key -> layout?.shown?.contains(key) ?: true }
    val d = detail ?: return
    val e = edit ?: return
    val row = d.row
    val act: (Samu.() -> Unit) -> Unit = { action ->
        scope.launch {
            withContext(Dispatchers.IO) { Store.get(context).action() }
            changed()
        }
    }
    if (moving) {
        MovePicker(row.projectId, onPick = { target -> act { moveTask(id, target) }; moving = false }, close = { moving = false })
    }
    if (blocking) {
        BlockerPicker(id, onPick = { target -> act { runCatching { addBlocker(id, target) } }; blocking = false }, close = { blocking = false })
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        TextButton(onClick = back) { Text(tr("‹ Zurück")) }
        ProjectLine(row.place) { moving = true }
        if (e.descendants > 0u) Text(tr("Mit {descendants} Unteraufgaben; sie wandern beim Verschieben mit.", "descendants" to e.descendants), color = t.muted, fontSize = 12.sp)
        EditableText(row.title, tr("Titel"), single = true, big = true) { text -> if (text.isNotBlank()) act { setTitle(id, text) } }
        IdLine(id, d.createdMs)
        if (show("properties")) PropertiesPart(id, tick, act)
        if (show("availability")) {
            val (bg, fg, text) = when {
                row.done -> Triple(t.rowline, t.muted, tr("erledigt"))
                row.dropped -> Triple(t.rowline, t.muted, tr("verworfen"))
                row.available -> Triple(t.okBg, t.ok, tr("✓ verfügbar"))
                else -> Triple(t.waitBg, t.wait, tr("nicht verfügbar"))
            }
            Column(Modifier.fillMaxWidth().background(bg, RoundedCornerShape(8.dp)).padding(10.dp)) {
                Text(text, color = fg, fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
                if (!row.available && !row.done) d.reasons.forEach { Text("• " + Words.reason(it), color = fg, fontSize = 13.sp) }
            }
        }
        if (show("links")) BlockersPart(id, row, tick, act, open) { blocking = true }
        if (show("links")) HeldUpPart(id, tick, act, open)
        if (listOf("defer", "due", "planned", "repeat").any(show)) Column {
            if (show("defer")) DateLine(tr("Start"), row.defer) { v -> act { setDate(id, "defer", v) } }
            if (show("due")) DateLine(tr("Fällig"), row.due) { v -> act { setDate(id, "due", v) } }
            if (show("due") && row.due != null) DueWarningLine(id, e, act)
            if (show("planned")) DateLine(tr("Geplant"), row.planned) { v -> act { setDate(id, "planned", v) } }
            if (show("repeat")) RepeatLine(id, e.repeat, changed)
        }
        if (show("series") && row.habit != null) SeriesPart(id, tick, act)
        if (show("note")) {
            Text(tr("NOTIZ"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
            NoteField(e.note ?: "", { v -> act { setNote(id, v) } }, open, rendered = layout?.noteRendered == true)
        }
        EditChips(id, e, act, show)
        if (show("clock")) ClockPart(id, row, d.estimate, tick, act)
        if (show("reminders") && !row.done && !row.dropped) RemindersPart(id, row, tick, act)
        NotesPart(id, tick, act, open, showComments = show("comments"), showFiles = show("attachments"))
        layout?.let { l -> FoldLine(l) { on -> act { setInspectorUnfolded(on) } } }
        if (row.pending) Text(tr("Wartet im Ausgang auf den nächsten Sync."), color = t.wait, fontSize = 13.sp)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (row.done || row.dropped) {
                OutlinedButton(onClick = { act { reopen(id) } }) { Text(tr("Wieder öffnen")) }
            } else {
                Button(onClick = { act { complete(id) }; back() }) { Text(tr("Erledigen")) }
                OutlinedButton(onClick = { act { dropTask(id) } }) { Text(tr("Verwerfen")) }
            }
            OutlinedButton(onClick = { act { setFlag(id, !row.flagged) } }) { Text(if (row.flagged) tr("Markierung weg") else tr("⚑ Markieren")) }
        }
        DeleteButton(id, changed, back)
        Spacer(Modifier.heightIn(min = 48.dp))
    }
}

/**
 * The quiet line under the parts a scope shows (ADR 0072 §3): what is
 * folded and has a value first, by name, then how many more; "Weniger"
 * once unfolded.
 */
@Composable
private fun FoldLine(layout: TaskLayout, unfold: (Boolean) -> Unit) {
    val t = LocalTokens.current
    val text = when {
        layout.unfolded -> tr("Weniger")
        layout.valued.isNotEmpty() ->
            tr("Mehr: ") + layout.valued.joinToString(", ") { Words.part(it) } +
                if (layout.rest > 0u) tr(" · {rest} weitere", "rest" to layout.rest) else ""
        layout.rest > 0u -> tr("Mehr ({rest})", "rest" to layout.rest)
        else -> return
    }
    Text(
        text, color = if (layout.valued.isNotEmpty()) t.fg else t.muted, fontSize = 13.sp,
        modifier = Modifier.fillMaxWidth().clickable { unfold(!layout.unfolded) }.padding(vertical = 8.dp),
    )
}

/**
 * The time on a task (ADR 0046 §8): the sum against the estimate, start or
 * stop, and its clocks. One running elsewhere ends when this one starts;
 * samu-mobile sees to that, as the server does.
 */
@Composable
private fun ClockPart(id: String, row: TaskRow, estimate: String?, tick: Int, act: (Samu.() -> Unit) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var clocks by remember { mutableStateOf<List<ClockRow>>(emptyList()) }
    var running by remember { mutableStateOf<RunningClock?>(null) }
    LaunchedEffect(tick, id) {
        withContext(Dispatchers.IO) {
            val samu = Store.get(context)
            clocks = runCatching { samu.clocks(id) }.getOrDefault(emptyList())
            running = runCatching { samu.runningClock() }.getOrNull()
        }
    }
    val mine = running?.ownerId == id
    val total = clocks.sumOf { it.seconds }
    Text(tr("ZEIT"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
    Text(
        Words.length(total) + tr(" geclockt") + (estimate?.let { tr(" · Schätzung ") + Words.estimate(it) } ?: ""),
        color = t.fg, fontSize = 14.sp,
    )
    if (mine) {
        Button(onClick = { act { clockOut() } }) { Text(tr("Stopp")) }
    } else if (!row.done && !row.dropped) {
        OutlinedButton(onClick = { act { clockIn(id) } }) {
            Text(if (running != null) tr("Hier starten (stoppt „{title}“)", "title" to running?.title) else tr("Uhr|Start"))
        }
    }
    clocks.take(5).forEach { c ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(Words.clockSpan(c), color = if (c.endMs == null) t.ok else t.fg, fontSize = 13.sp, modifier = Modifier.weight(1f))
            Text(Words.length(c.seconds), color = t.fg, fontSize = 13.sp)
        }
    }
    if (clocks.size > 5) Text(tr("und {n} ältere", "n" to clocks.size - 5), color = t.muted, fontSize = 12.sp)
}

/**
 * The reminders set on a task (ADR 0030): a due date reminds by itself,
 * the buttons add one before a date or at an instant of its own.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun RemindersPart(id: String, row: TaskRow, tick: Int, act: (Samu.() -> Unit) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var set by remember { mutableStateOf<List<ReminderRow>>(emptyList()) }
    LaunchedEffect(tick, id) {
        set = withContext(Dispatchers.IO) { runCatching { Store.get(context).reminders(id) }.getOrDefault(emptyList()) }
    }
    Text(tr("ERINNERUNGEN"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
    if (row.due != null) Text(tr("Zur Fälligkeit von selbst."), color = t.muted, fontSize = 13.sp)
    set.forEach { r ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(Words.reminder(r), color = t.fg, fontSize = 14.sp, modifier = Modifier.weight(1f))
            TextButton(onClick = { act { deleteReminder(r.id) } }) { Text(tr("Entfernen")) }
        }
    }
    val add: (Anchor, Long?, Long?) -> Unit = { anchor, at, before -> act { addReminder(id, anchor, at, before) } }
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (row.due != null) {
            OutlinedButton(onClick = { add(Anchor.DUE, null, 60) }) { Text(tr("1 h vor fällig")) }
            OutlinedButton(onClick = { add(Anchor.DUE, null, 1440) }) { Text(tr("1 Tag vor fällig")) }
        }
        if (row.planned != null) OutlinedButton(onClick = { add(Anchor.PLANNED, null, null) }) { Text(tr("Zum Plan")) }
        OutlinedButton(onClick = { pickInstant(context) { ms -> add(Anchor.AT, ms, null) } }) { Text(tr("Zeitpunkt …")) }
        OutlinedButton(onClick = {
            android.app.TimePickerDialog(context, { _, hour, minute ->
                act { addDailyReminder(id, (hour * 60 + minute).toLong()) }
            }, 20, 0, true).show()
        }) { Text(tr("Täglich um …")) }
    }
}

/**
 * The tasks this one waits for (ae5a3206): a tap opens one, ✕ takes it
 * away, „+ Blocker“ picks another. Hidden on a closed task without any.
 */
@Composable
private fun BlockersPart(
    id: String, row: TaskRow, tick: Int, act: (Samu.() -> Unit) -> Unit, open: (String) -> Unit, add: () -> Unit,
) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var set by remember { mutableStateOf<List<BlockerRow>>(emptyList()) }
    LaunchedEffect(tick, id) {
        set = withContext(Dispatchers.IO) { runCatching { Store.get(context).blockers(id) }.getOrDefault(emptyList()) }
    }
    val closed = row.done || row.dropped
    if (set.isEmpty() && closed) return
    Column {
        Text(tr("BLOCKIERT DURCH"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        set.forEach { b ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    b.title + (b.place?.let { "  · $it" } ?: ""),
                    color = if (b.done) t.muted else t.fg, fontSize = 14.sp,
                    textDecoration = if (b.done) TextDecoration.LineThrough else null,
                    modifier = Modifier.weight(1f).clickable { open(b.taskId) }.padding(vertical = 8.dp),
                )
                b.linkId?.let { link -> TextButton(onClick = { act { removeBlocker(link) } }) { Text("✕") } }
            }
        }
        if (!closed) TextButton(onClick = add) { Text(tr("+ Blocker")) }
    }
}

/**
 * The open tasks that wait for this one (53a0d8e9, „Hält auf“): a tap opens
 * one, ✕ takes its blocker away. Hidden when none waits.
 */
@Composable
private fun HeldUpPart(id: String, tick: Int, act: (Samu.() -> Unit) -> Unit, open: (String) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var set by remember { mutableStateOf<List<BlockerRow>>(emptyList()) }
    LaunchedEffect(tick, id) {
        set = withContext(Dispatchers.IO) { runCatching { Store.get(context).heldUp(id) }.getOrDefault(emptyList()) }
    }
    if (set.isEmpty()) return
    Column {
        Text(tr("HÄLT AUF"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        set.forEach { b ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    b.title + (b.place?.let { "  · $it" } ?: ""),
                    color = t.fg, fontSize = 14.sp,
                    modifier = Modifier.weight(1f).clickable { open(b.taskId) }.padding(vertical = 8.dp),
                )
                b.linkId?.let { link -> TextButton(onClick = { act { removeBlocker(link) } }) { Text("✕") } }
            }
        }
    }
}

/** A date, then a time, with the system's pickers; `then` gets milliseconds. */
private fun pickInstant(context: android.content.Context, then: (Long) -> Unit) {
    val now = java.time.LocalDateTime.now().plusHours(1).withMinute(0)
    android.app.DatePickerDialog(context, { _, year, month, day ->
        android.app.TimePickerDialog(context, { _, hour, minute ->
            val at = java.time.LocalDateTime.of(year, month + 1, day, hour, minute)
                .atZone(java.time.ZoneId.systemDefault()).toInstant().toEpochMilli()
            then(at)
        }, now.hour, now.minute, true).show()
    }, now.year, now.monthValue - 1, now.dayOfMonth).show()
}

/** The offered [name] in place of the whole #…/@… word, the cursor behind it. */
internal fun accepted(field: TextFieldValue, offer: Offer, name: String): TextFieldValue {
    val text = field.text
    val start = offer.start.toInt().coerceIn(0, text.length)
    val end = offer.end.toInt().coerceIn(start, text.length)
    val written = if (name.any { it.isWhitespace() }) "${offer.sign}\"$name\"" else offer.sign + name
    val rest = text.substring(end).trimStart()
    val caret = start + written.length + 1
    return TextFieldValue(text.substring(0, start) + written + " " + rest, TextRange(caret))
}

/** The line without what a chip stands for, one space left where it was. */
private fun withoutSpans(field: TextFieldValue, spans: List<Span>): TextFieldValue {
    var text = field.text
    var caret = text.length
    for (span in spans.sortedByDescending { it.start }) {
        val start = span.start.toInt().coerceIn(0, text.length)
        val end = span.end.toInt().coerceIn(start, text.length)
        text = text.removeRange(start, end)
        // Two spaces meet where the piece was, or one leads the line.
        if (start < text.length && text[start] == ' ' && (start == 0 || text[start - 1] == ' ')) {
            text = text.removeRange(start, start + 1)
        }
        caret = start
    }
    return TextFieldValue(text, TextRange(caret.coerceAtMost(text.length)))
}

/**
 * Capture as a sheet from below, with the parser's preview (ADR 0028 §7);
 * with `parent`, the new task goes under it (ADR 0062 §2).
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun CaptureSheet(project: String?, shared: Share? = null, parent: TaskRow? = null, onDone: (String?) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    // A shared text, taken apart by samu-core; it is never parsed (ADR 0013 §2).
    val draft = remember(shared) { shared?.let { Store.get(context).shareDraft(it.subject, it.text) } }
    var title by remember(draft) { mutableStateOf(draft?.title.orEmpty()) }
    var field by remember { mutableStateOf(TextFieldValue("")) }
    val text = field.text
    var preview by remember { mutableStateOf<CapturePreview?>(null) }
    var offer by remember { mutableStateOf<Offer?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val focus = remember { FocusRequester() }
    // The account's templates as buttons (ADR 0073 §3).
    var templates by remember { mutableStateOf<List<TemplateRow>>(emptyList()) }
    LaunchedEffect(Unit) {
        templates = withContext(Dispatchers.IO) { runCatching { Store.get(context).templates() }.getOrDefault(emptyList()) }
    }
    LaunchedEffect(Unit) { if (draft == null) focus.requestFocus() }
    LaunchedEffect(text) {
        delay(120)
        preview = if (text.isBlank()) null else withContext(Dispatchers.IO) {
            runCatching { Store.get(context).preview(text, project) }.getOrNull()
        }
    }
    // Projects or tags for the #…/@… word at the cursor, from its sign on.
    LaunchedEffect(text, field.selection) {
        val cursor = field.selection.start.toUInt()
        offer = withContext(Dispatchers.IO) {
            runCatching { Store.get(context).captureOffer(text, cursor) }.getOrNull()
        }
    }
    val ready = if (draft != null) title.isNotBlank() else text.isNotBlank()
    val create: () -> Unit = {
        if (ready) {
            scope.launch {
                try {
                    val id = withContext(Dispatchers.IO) {
                        val samu = Store.get(context)
                        when {
                            draft != null -> samu.captureShared(title, text, draft.note, draft.links, project)
                            parent != null -> samu.captureUnder(text, parent.id)
                            else -> samu.capture(text, project)
                        }
                    }
                    onDone(id)
                } catch (e: Exception) {
                    error = said(e)
                }
            }
        }
    }
    ModalBottomSheet(onDismissRequest = { onDone(null) }, sheetState = sheet, containerColor = t.pane) {
        Column(Modifier.fillMaxWidth().imePadding().padding(16.dp, 0.dp, 16.dp, 24.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            val p = preview
            if (draft != null) {
                val rest = p?.title?.trim().orEmpty()
                val whole = if (rest.isEmpty()) title.trim() else "${title.trim()} $rest"
                Text(tr("Geteilt · „{whole}“ in {project}", "whole" to whole, "project" to (p?.project ?: "Inbox")), color = t.muted, fontSize = 13.sp)
                OutlinedTextField(
                    title, { title = it }, label = { Text(tr("Titel")) }, singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            } else {
                // A project the line names wins over the parent (ADR 0062 §2).
                val named = p?.chips?.any { it.kind == "project" && it.spans.isNotEmpty() } == true
                val where = if (parent != null && !named) tr("unter „{title}“", "title" to parent.title) else tr("in {project}", "project" to (p?.project ?: "Inbox"))
                Text(
                    when {
                        p != null && p.title.isNotBlank() -> "„${p.title}“ $where"
                        parent != null -> tr("Unteraufgabe {where}", "where" to where)
                        else -> tr("Neue Aufgabe")
                    },
                    color = t.muted, fontSize = 13.sp,
                )
            }
            OutlinedTextField(
                field, { field = it },
                placeholder = { Text(if (draft != null) tr("#projekt @tag d:fr – ergänzt den Titel") else "#projekt @tag ! !A d:fr ~15m // Notiz") },
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { create() }),
            )
            if (templates.isNotEmpty()) {
                // A tap puts `/name` into the line, so the preview shows
                // what comes of it; one template per line.
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    templates.forEach { tpl ->
                        val call = "/" + tpl.title
                        val on = Regex("(^|\\s)" + Regex.escape(call) + "(\\s|$)", RegexOption.IGNORE_CASE).containsMatchIn(text)
                        Text(call, color = if (on) t.accent else t.muted, fontSize = 13.sp,
                            modifier = Modifier
                                .border(1.dp, if (on) t.accent else t.border, RoundedCornerShape(999.dp))
                                .clickable {
                                    if (!on) {
                                        val next = (text.trimEnd() + " " + call).trimStart()
                                        field = TextFieldValue(next, TextRange(next.length))
                                    }
                                }
                                .padding(12.dp, 6.dp))
                    }
                }
            }
            offer?.let { o ->
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    o.items.forEach { name ->
                        Text(o.sign + name, color = t.accent, fontSize = 13.sp,
                            modifier = Modifier
                                .border(1.dp, t.accent, RoundedCornerShape(999.dp))
                                .clickable { field = accepted(field, o, name); offer = null }
                                .padding(12.dp, 6.dp))
                    }
                }
            }
            if (p != null) {
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    p.chips.forEach { chip ->
                        val (bg, fg) = when (chip.kind) {
                            "project" -> t.select to t.accent
                            "tag", "new_tag" -> t.okBg to t.ok
                            "due" -> t.soonBg to t.soon
                            else -> t.rowline to t.fg
                        }
                        // A chip the line names comes out of it with a tap.
                        val removable = chip.spans.isNotEmpty()
                        Text(if (removable) Words.chip(chip) + "  ×" else Words.chip(chip), color = fg, fontSize = 12.sp,
                            modifier = Modifier.background(bg, RoundedCornerShape(999.dp))
                                .then(if (removable) Modifier.clickable { field = withoutSpans(field, chip.spans) } else Modifier)
                                .padding(10.dp, 3.dp))
                    }
                }
                p.warnings.forEach { Text("• " + Words.warning(it), color = t.wait, fontSize = 13.sp) }
            }
            if (draft != null) {
                draft.note?.let { note ->
                    Text(tr("Notiz: ") + note.lineSequence().first().take(80) + if (note.contains('\n') || note.length > 80) " …" else "",
                        color = t.muted, fontSize = 13.sp)
                }
                draft.links.forEach { Text("↗ $it", color = t.accent, fontSize = 13.sp, maxLines = 1) }
            }
            error?.let { Text(it, color = t.overdue) }
            Button(onClick = create, enabled = ready, modifier = Modifier.align(Alignment.End)) { Text(tr("Anlegen")) }
        }
    }
}
