// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.ui.text.style.TextOverflow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.lifecycleScope
import androidx.work.WorkInfo
import androidx.work.WorkManager
import de.samudo.app.mobile.AppointmentRow
import de.samudo.app.mobile.BarEntry
import de.samudo.app.mobile.Status
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Bumped to draw again: after a sync, a change, or coming back to the app. */
private val refresh = mutableIntStateOf(0)

/**
 * Draws the open app again, from anywhere: a receiver that changed what it
 * shows (a scope from Tasker, ADR 0043 §8; a recording from the ring) calls
 * it, on the main thread.
 */
fun redrawApp() {
    android.os.Handler(android.os.Looper.getMainLooper()).post { refresh.intValue++ }
}

/** Text shared to the app (ADR 0013), until the sheet for it is closed. */
data class Share(val subject: String?, val text: String)

private val pendingShare = mutableStateOf<Share?>(null)

/** A tap in the widget: capture, or a task by its id. */
sealed interface Open {
    data object Capture : Open
    data object Inbox : Open
    data class Task(val id: String) : Open
    /** An occurrence of an appointment, from its reminder (ADR 0101 §7). */
    data class Appointment(val key: String) : Open
}

private val pendingOpen = mutableStateOf<Open?>(null)

/** The scheme of an object's address (ADR 0075 §4) and the id in it. */
private const val SAMU_SCHEME = "samu"
private val UUID = Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // The phone's language may have changed since the store opened.
        I18n.init(this, Store.get(this))
        Sync.periodic(this)
        askToNotify()
        takeShare(intent)
        takeWidget(intent)
        takeAddress(intent)
        setContent { SamuTheme { App() } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        takeShare(intent)
        takeWidget(intent)
        takeAddress(intent)
    }

    /** Reminders need leave to notify from Android 13 on (ADR 0030); asked once at the start. */
    private val notifyAsked = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    private fun askToNotify() {
        Reminders.channel(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            notifyAsked.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    /** `samu-widget:capture`, `:inbox` or `:task/<id>`, from [TodayWidget] or the launcher. */
    private fun takeWidget(intent: Intent?) {
        val data = intent?.data ?: return
        if (data.scheme != WIDGET_SCHEME) return
        val part = data.schemeSpecificPart.orEmpty()
        pendingOpen.value = when {
            part == CAPTURE -> Open.Capture
            part == INBOX -> Open.Inbox
            part.startsWith("$TASK/") -> Open.Task(part.removePrefix("$TASK/"))
            part.startsWith("$APPOINTMENT/") -> Open.Appointment(part.removePrefix("$APPOINTMENT/"))
            else -> null
        }
    }

    /** `samu:<uuid>` from any app (ADR 0075 §4): the task, like a tap in the widget. */
    private fun takeAddress(intent: Intent?) {
        val data = intent?.data ?: return
        if (intent.action != Intent.ACTION_VIEW || !data.scheme.equals(SAMU_SCHEME, ignoreCase = true)) return
        val id = data.schemeSpecificPart.orEmpty().trim('/').lowercase()
        if (UUID.matches(id)) pendingOpen.value = Open.Task(id)
    }

    /** A share waits through the sign-in, if there is one first. */
    private fun takeShare(intent: Intent?) {
        if (intent?.action != Intent.ACTION_SEND || intent.type?.startsWith("text/") != true) return
        val text = intent.getStringExtra(Intent.EXTRA_TEXT).orEmpty()
        val subject = intent.getStringExtra(Intent.EXTRA_SUBJECT)
        if (text.isBlank() && subject.isNullOrBlank()) return
        pendingShare.value = Share(subject, text)
    }

    override fun onResume() {
        super.onResume()
        Sync.now(this)
        Reminders.arm(this)
        Clocking.refresh(this)
        refresh.intValue++
    }

    /** What changed in the app shows in the widget too. */
    override fun onPause() {
        super.onPause()
        val context = applicationContext
        lifecycleScope.launch { Widgets.refresh(context) }
    }
}

/** A fixed view as an entry of the bar (ADR 0102 §1). */
fun viewEntry(key: String, title: String) = BarEntry("view", key, title, null, false)

private val INBOX_VIEW = viewEntry("inbox", "Inbox")
private val AVAILABLE_VIEW get() = viewEntry("available", tr("Verfügbar"))

/**
 * Where the screen is: an entry of the bar, maybe a project in it, maybe
 * a task, or the settings.
 */
data class Place(
    val entry: BarEntry,
    val project: Pair<String, String>? = null,
    val task: String? = null,
    val settings: Boolean = false,
    /** A folder or project to open in the settings (ADR 0085 §6). */
    val settingsPlace: String? = null,
)

@Composable
fun App() {
    val context = LocalContext.current
    // The status with the folder it was read from: after a switch nothing
    // is drawn from the account left (ADR 0107 §3). Never a Samu held here:
    // Store.get gives the active one.
    var status by remember { mutableStateOf<Pair<String, Status>?>(null) }
    val tick = refresh.intValue
    val slot = Accounts.current.value ?: Store.active(context)

    // A finished sync draws again.
    LaunchedEffect(Unit) {
        WorkManager.getInstance(context).getWorkInfosForUniqueWorkFlow(Sync.NOW).collect { infos ->
            if (infos.any { it.state == WorkInfo.State.SUCCEEDED || it.state == WorkInfo.State.ENQUEUED }) {
                refresh.intValue++
            }
        }
    }
    LaunchedEffect(tick, slot) {
        status = withContext(Dispatchers.IO) { Store.active(context) to Store.get(context).status() }
    }
    LaunchedEffect(slot) {
        Details.on.value = withContext(Dispatchers.IO) { runCatching { Store.get(context).details() }.getOrDefault(false) }
    }

    // The accounts on the phone (ADR 0107 §4), for the menu and the sign-in.
    var accounts by remember { mutableStateOf<List<AccountLine>>(emptyList()) }
    LaunchedEffect(tick) { accounts = withContext(Dispatchers.IO) { Accounts.list(context) } }

    val (read, current) = status ?: return
    if (read != slot) return
    // Another account starts the main screen anew: no place of the old one stays.
    androidx.compose.runtime.key(slot) {
        if (!current.signedIn) {
            LoginScreen(
                current,
                others = accounts.filter { !it.active && it.account != null },
                // While adding, or in a folder that never held an account
                // (the app restarted while adding): back to another one.
                cancel = if (Accounts.addingFrom.value != null || (current.account == null && accounts.any { !it.active && it.account != null })) {
                    { Accounts.cancelAdding(context) }
                } else {
                    null
                },
            ) { Accounts.signedIn(context); refresh.intValue++ }
        } else {
            Main(current, tick, accounts)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Main(status: Status, tick: Int, accounts: List<AccountLine>) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var place by remember { mutableStateOf(Place(AVAILABLE_VIEW)) }
    // The bar's entries, without the hidden ones (ADR 0102 §1, §3); their
    // titles come from samu-mobile in the app's language, so a change of
    // it loads them again.
    val (bar, _) = loaded(tick, "bar" to I18n.lang) { it.bar() }
    val shown = bar?.filter { !it.hidden }
    // A fixed view's key, `perspective`, `project` or `tag`.
    val view = if (place.entry.kind == "view") place.entry.key else place.entry.kind
    // A project of the bar is its entry's screen, not one opened in it.
    val ownProject = place.entry.kind == "project" && place.project?.first == place.entry.key
    var capturing by remember { mutableStateOf(false) }
    // An appointment's sheet, opened from its reminder.
    var appointment by remember { mutableStateOf<AppointmentRow?>(null) }
    // The tag chip stays when the tab changes; the search keeps its words.
    var tag by remember { mutableStateOf<String?>(null) }
    var query by remember { mutableStateOf("") }
    // A line being typed in the project tree (ADR 0085 §2).
    var treeDraft by remember { mutableStateOf<TreeDraft?>(null) }
    // The head's menu (ADR 0102 §6).
    var menu by remember { mutableStateOf(false) }
    // Pulled down: until the sync is through, or a while when it cannot start.
    var pulling by remember { mutableStateOf(false) }
    LaunchedEffect(pulling) {
        if (!pulling) return@LaunchedEffect
        withTimeoutOrNull(8_000) {
            WorkManager.getInstance(context).getWorkInfosForUniqueWorkFlow(Sync.NOW)
                .first { infos -> infos.all { it.state.isFinished } }
        }
        pulling = false
    }
    LaunchedEffect(place.entry, place.project, place.task, place.settings) { treeDraft = null }
    // Forecast as a list or as the calendar (ADR 0047 §11).
    var forecastMode by rememberSaveable { mutableStateOf(ForecastMode.LIST) }
    val changed: () -> Unit = { refresh.intValue++; Sync.afterChange(context); Reminders.arm(context); Clocking.refresh(context) }
    val opened = pendingOpen.value
    LaunchedEffect(opened) {
        when (opened) {
            Open.Capture -> capturing = true
            Open.Inbox -> place = Place(INBOX_VIEW)
            is Open.Task -> place = place.copy(task = opened.id)
            is Open.Appointment -> appointment = withContext(Dispatchers.IO) {
                runCatching { Store.get(context).appointment(opened.key) }.getOrNull()
            }
            null -> {}
        }
        pendingOpen.value = null
    }

    // A new view, tab, tag or search ends the picking (ADR 0052 §6).
    LaunchedEffect(place.entry, place.project, place.settings, tag, query, forecastMode) { Picks.clear() }
    BackHandler(enabled = Picks.active) { Picks.clear() }
    BackHandler(enabled = !Picks.active && (place.task != null || (place.project != null && !ownProject) || place.settings)) {
        place = when {
            place.settings -> place.copy(settings = false, settingsPlace = null)
            place.task != null -> place.copy(task = null)
            else -> place.copy(project = null)
        }
    }

    Scaffold(
        containerColor = t.bg,
        snackbarHost = { UndoHost(changed) },
        bottomBar = {
            if (shown != null) {
                Bar(shown, place.entry.takeIf { place.task == null && !place.settings }) { entry ->
                    place = if (entry.kind == "project") Place(entry, project = entry.key to entry.title) else Place(entry)
                }
            }
        },
        floatingActionButton = {
            if (place.task == null && !place.settings) {
                ExtendedFloatingActionButton(
                    onClick = { capturing = true },
                    containerColor = t.accent,
                    contentColor = t.onAccent,
                    shape = RoundedCornerShape(16.dp),
                ) { Text(tr("+  Erfassen"), fontWeight = FontWeight.SemiBold) }
            }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            val title = when {
                place.settings -> tr("Einstellungen")
                place.task != null -> ""
                place.project != null -> place.project!!.second
                else -> place.entry.title
            }
            // A list of tasks, with Details and the tag chips; not the
            // tree, the search, Stuck, Review or the calendar.
            val lists = !place.settings && place.task == null && (
                place.project != null || (view !in setOf("projects", "search", "stuck", "review") &&
                    !(view == "forecast" && forecastMode != ForecastMode.LIST))
                )
            // The list's key for Anordnen (ADR 0125 §1); none where there is no button.
            val arrangeKey: String? = when {
                place.settings || place.task != null -> null
                place.project != null -> "project:${place.project!!.first}"
                view in setOf("inbox", "available", "today", "flagged", "search") -> view
                view == "perspective" || view == "tag" -> "$view:${place.entry.key}"
                else -> null
            }
            val picking = Picks.active && place.task == null && !place.settings
            val scopes = rememberScopeState(tick)
            if (picking) {
                PickBar(changed) { id -> place = place.copy(task = id) }
            } else if (title.isNotEmpty()) {
                // One line: title, the scope as a chip, a menu (ADR 0102 §6).
                Row(Modifier.fillMaxWidth().padding(16.dp, 12.dp, 4.dp, 4.dp), verticalAlignment = Alignment.CenterVertically) {
                  // The title takes what the chip leaves; the menu stays at the edge.
                  Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        title, fontSize = 24.sp, fontWeight = FontWeight.Bold, color = t.fg,
                        maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
                    )
                    // Something waits in the outbox.
                    if (status.outbox > 0u) {
                        Box(Modifier.padding(start = 6.dp).size(8.dp).background(t.wait, CircleShape))
                    }
                    Spacer(Modifier.width(10.dp))
                    if (!place.settings) ScopeChip(scopes, changed) { place = place.copy(settings = true) }
                  }
                    arrangeKey?.let { key ->
                        TextButton(onClick = { Arranging.open = key }) { Text("⇅", fontSize = 18.sp, color = t.fg) }
                    }
                    Box {
                        TextButton(onClick = { menu = true }) { Text("⋮", fontSize = 20.sp, color = t.fg) }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            if (lists) {
                                // Details as a second line (ADR 0053 §1), for every list.
                                DropdownMenuItem(
                                    text = { Text(if (Details.on.value) tr("✓ Details") else "Details") },
                                    onClick = { menu = false; scope.launch { Details.toggle(Store.get(context)) } },
                                )
                            }
                            if (lists) {
                                // Picking from the header, none picked yet (ADR 0062 §5).
                                DropdownMenuItem(text = { Text(tr("Auswählen")) }, onClick = { menu = false; Picks.start() })
                            }
                            if (!place.settings && place.task == null && place.project == null && view == "projects") {
                                // A new project or folder at the top (ADR 0085 §2).
                                DropdownMenuItem(text = { Text(tr("Neues Projekt")) }, onClick = { menu = false; treeDraft = TreeDraft("project") })
                                DropdownMenuItem(text = { Text(tr("Neuer Ordner")) }, onClick = { menu = false; treeDraft = TreeDraft("folder") })
                            }
                            DropdownMenuItem(
                                text = { Text(if (status.outbox > 0u) tr("Sync · {outbox} im Ausgang", "outbox" to status.outbox) else tr("Sync")) },
                                onClick = { menu = false; Sync.now(context) },
                            )
                            if (!place.settings) {
                                DropdownMenuItem(text = { Text(tr("Einstellungen")) }, onClick = { menu = false; place = place.copy(settings = true) })
                            }
                            // Switching quickly, with more than one account (ADR 0107 §4).
                            accounts.filter { !it.active }.forEach { other ->
                                DropdownMenuItem(
                                    text = { Text(tr("⇄ {label}", "label" to other.label)) },
                                    onClick = { menu = false; Accounts.switch(context, other.slot) },
                                )
                            }
                        }
                    }
                }
            }
            // The active account, small under the title, once there are two.
            if (accounts.size > 1 && !picking && title.isNotEmpty()) {
                accounts.find { it.active }?.let {
                    Text(it.label, color = t.muted, fontSize = 12.sp, modifier = Modifier.padding(start = 16.dp, bottom = 4.dp))
                }
            }
            Banner(status)
            if (place.task == null && !place.settings) ScopeLine(scopes)
            // A perspective's chips under its title (ADR 0102 §4).
            if (view == "perspective" && place.project == null && place.task == null && !place.settings) {
                PerspectiveHead(tick, place.entry.key)
            }
            // A tag of the bar is its own filter; the search has its chips in
            // the field. In a list they scroll away with it (ADR 0102 §6).
            val tagChips = (lists && view != "tag") || (view == "forecast" && place.project == null && place.task == null && !place.settings)
            val chips: (@Composable () -> Unit)? =
                if (tagChips || arrangeKey != null) {
                    {
                        Column {
                            // The arrangement, when not the default (ADR 0125 §2).
                            if (arrangeKey != null) ArrangeLine(tick, arrangeKey)
                            if (tagChips) TagChips(tick, tag) { tag = it }
                        }
                    }
                } else {
                    null
                }
            if (view == "forecast" && forecastMode != ForecastMode.LIST && place.project == null && place.task == null && !place.settings) {
                chips?.invoke()
            }
            // A link in a note leads to a task or a project (ADR 0077 §4).
            val openTask: (String) -> Unit = { id ->
                val project = runCatching { Store.get(context).places(true) }.getOrDefault(emptyList())
                    .find { it.id == id && it.kind == "project" }
                place = if (project != null) place.copy(task = null, project = id to project.title) else place.copy(task = id)
            }
            // Pulling down syncs (ADR 0102 §6), instead of a button.
            PullToRefreshBox(
                isRefreshing = pulling,
                onRefresh = { pulling = true; Sync.now(context) },
                modifier = Modifier.weight(1f),
            ) {
                when {
                    place.settings -> SettingsScreen(tick, changed, place.settingsPlace) { place = place.copy(settings = false, settingsPlace = null) }
                    place.task != null -> TaskScreen(place.task!!, tick, changed, openTask) { place = place.copy(task = null) }
                    place.project != null -> {
                        val id = place.project!!.first
                        TaskList(tick, id to tag, { it.project(id, tag) }, changed, open = openTask, head = chips)
                    }
                    view == "inbox" -> TaskList(tick, "inbox" to tag, { it.inbox(tag) }, changed, open = openTask, head = chips)
                    view == "available" -> TaskList(tick, "available" to tag, { it.available(tag) }, changed, open = openTask, head = chips)
                    view == "today" -> TaskList(tick, "today" to tag, { it.todayList(tag) }, changed, tr("Nichts für heute geplant."), openTask, chips)
                    view == "flagged" -> TaskList(tick, "flagged" to tag, { it.flagged(tag) }, changed, tr("Nichts markiert."), openTask, chips)
                    view == "habits" -> TaskList(tick, "habits" to tag, { it.habitList(tag) }, changed, tr("Keine Habits."), openTask, chips)
                    view == "stuck" || view == "review" -> ProjectViews(
                        tick, view == "review", changed,
                        openProject = { id, name -> place = place.copy(project = id to name) },
                        open = openTask,
                    )
                    view == "perspective" -> {
                        val id = place.entry.key
                        TaskList(tick, "perspective-$id" to tag, { it.perspective(id, tag) }, changed, open = openTask, head = chips)
                    }
                    view == "tag" -> {
                        val id = place.entry.key
                        TaskList(tick, "tag-$id", { it.tagged(id) }, changed, open = openTask, head = chips)
                    }
                    view == "forecast" -> Column {
                        ForecastModes(forecastMode) { forecastMode = it }
                        Box(Modifier.weight(1f)) {
                            if (forecastMode == ForecastMode.LIST) {
                                ForecastList(tick, tag, changed, openTask, chips)
                            } else {
                                CalendarView(tick, forecastMode == ForecastMode.WEEK, { forecastMode = ForecastMode.DAY }, openTask)
                            }
                        }
                    }
                    view == "search" -> SearchScreen(tick, tag, { tag = it }, query, { query = it }, changed, openTask) { place = Place(it) }
                    else -> ProjectTree(
                        tick, treeDraft, { treeDraft = it }, changed,
                        open = { id, name -> place = place.copy(project = id to name) },
                        settings = { id -> place = place.copy(settings = true, settingsPlace = id) },
                    )
                }
            }
        }
    }
    appointment?.let { AppointmentSheet(it) { appointment = null } }
    Arranging.open?.let { key -> ArrangeSheet(key, changed) { Arranging.open = null } }
    val share = pendingShare.value
    Closing.ask.value?.let { CloseSheet(it, changed) }
    TaskMenus.editing.value?.let { row ->
        EditSheet(row) { saved ->
            TaskMenus.editing.value = null
            if (saved) changed()
        }
    }
    val under = TaskMenus.under.value
    if (capturing || share != null || under != null) {
        CaptureSheet(project = place.project?.first, shared = share, parent = under.takeIf { share == null }, onDone = { id ->
            capturing = false
            pendingShare.value = null
            TaskMenus.under.value = null
            if (id != null) changed()
        })
    }
}

/**
 * The last error of the sync and a zone the server does not name. A full
 * outbox is a dot at the title and its count in the menu (ADR 0102 §6).
 */
@Composable
private fun Banner(status: Status) {
    val t = LocalTokens.current
    val lines = buildList {
        status.lastError?.let { add(it) }
        if (!status.zoneFromServer) add(tr("Zone des Handys ({zone}); der Server nennt keine", "zone" to status.zone))
    }
    if (lines.isEmpty()) return
    Column(
        Modifier.fillMaxWidth().padding(16.dp, 4.dp, 16.dp, 8.dp)
            .background(t.waitBg, RoundedCornerShape(8.dp)).padding(12.dp, 8.dp),
    ) {
        lines.forEach { Text(it, color = t.wait, fontSize = 13.sp) }
    }
}
