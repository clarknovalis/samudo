package de.samudo.app

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.DeleteCount
import de.samudo.app.mobile.PlaceDetail
import de.samudo.app.mobile.PlaceRow
import de.samudo.app.mobile.Samu
import de.samudo.app.mobile.ScopeRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private fun kindWord(kind: String) = when (kind) {
    "folder" -> tr("Ordner")
    "project" -> tr("Projekt")
    else -> tr("Objekt|Tag")
}

private val PROJECT_KIND get() = listOf("parallel" to tr("Parallel"), "sequential" to tr("Sequenziell"), "single_actions" to tr("Einzelaufgaben"))
private val STATUS get() = listOf("active" to tr("Aktiv"), "on_hold" to tr("Ruht"), "completed" to tr("Erledigt"), "dropped" to tr("Verworfen"))
private val REVIEW get() = listOf<Pair<String?, String>>(
    null to tr("Keine"), "P1W" to tr("Jede Woche"), "P2W" to tr("Alle 2 Wochen"), "P1M" to tr("Jeden Monat"), "P3M" to tr("Alle 3 Monate"),
)

/**
 * Projects and folders, or tags (ADR 0037 §6): the tree, a tap opens the
 * object's own screen. New ones go through the outbox and open at once.
 */
@Composable
fun PlacesScreen(section: String, tick: Int, changed: () -> Unit, start: String? = null, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var places by remember { mutableStateOf<List<PlaceRow>>(emptyList()) }
    var open by remember { mutableStateOf(start) }
    var closed by remember { mutableStateOf(false) }
    val tags = section == "tags"
    LaunchedEffect(tick, closed) {
        places = withContext(Dispatchers.IO) { runCatching { Store.get(context).places(closed) }.getOrDefault(emptyList()) }
    }
    BackHandler(enabled = open != null) { open = null }
    open?.let { id ->
        PlaceScreen(id, tick, changed) { open = null }
        return
    }
    val shown = places.filter { (it.kind == "tag") == tags }
    fun make(kind: String) {
        scope.launch {
            val id = withContext(Dispatchers.IO) {
                runCatching {
                    val samu = Store.get(context)
                    when (kind) {
                        "project" -> samu.createProject(tr("Neues Projekt"), null)
                        "folder" -> samu.createFolder(tr("Neuer Ordner"), null)
                        else -> samu.createTag(tr("Neuer Tag"), null)
                    }
                }.getOrNull()
            }
            changed()
            open = id
        }
    }
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Einstellungen")) } }
        item {
            Text(
                if (tags) tr("TAGS") else tr("PROJEKTE UND ORDNER"),
                color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.7.sp,
            )
        }
        item {
            Row(
                Modifier.fillMaxWidth().padding(vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (tags) {
                    OutlinedButton(onClick = { make("tag") }) { Text(tr("+ Tag")) }
                } else {
                    OutlinedButton(onClick = { make("project") }) { Text(tr("+ Projekt")) }
                    OutlinedButton(onClick = { make("folder") }) { Text(tr("+ Ordner")) }
                    Spacer(Modifier.weight(1f))
                    Row(Modifier.clickable { closed = !closed }, verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(checked = closed, onCheckedChange = { closed = it })
                        Text(tr("Erledigte"), color = t.muted, fontSize = 13.sp)
                    }
                }
            }
        }
        if (shown.isEmpty()) {
            item { Text(if (tags) tr("Noch keine Tags.") else tr("Noch keine Projekte."), color = t.muted, fontSize = 14.sp) }
        }
        items(shown, key = { it.id }) { p -> TreeLine(p) { open = p.id } }
        item {
            Text(
                if (tags) {
                    tr("Ein Tag mit Kindern oder einer, der einem Projekt als Ablauf dient, lässt sich nicht löschen.")
                } else {
                    tr("Ein Sammelsurium ist ein Projekt der Art Einzelaufgaben. Neues landet oben; den Ort wählst du im Objekt.")
                },
                color = t.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 12.dp),
            )
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}

@Composable
private fun TreeLine(p: PlaceRow, open: () -> Unit) {
    val t = LocalTokens.current
    Row(
        Modifier.fillMaxWidth()
            .clickable(onClick = open)
            .padding(start = (p.depth.toInt() * 18).dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(
            when (p.kind) {
                "folder" -> "▸"
                "tag" -> "#"
                else -> "○"
            },
            color = t.muted, fontSize = 14.sp,
        )
        Text(
            // ADR 0069 §9: a quiet mark on what only its own scope shows.
            p.title + (p.ownScopeTitle?.let { " ◌ $it" } ?: if (p.scopeOnly) " ◌" else ""),
            color = if (p.closed) t.muted else t.fg,
            fontWeight = if (p.kind == "folder") FontWeight.SemiBold else FontWeight.Normal,
            textDecoration = if (p.closed) TextDecoration.LineThrough else null,
            fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        if (p.meta.isNotEmpty()) Text(p.meta, color = t.muted, fontSize = 12.sp)
        Text("›", color = t.muted, fontSize = 16.sp)
    }
}

/** A label above a control. */
@Composable
private fun Field(label: String, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(top = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(label, color = LocalTokens.current.muted, fontSize = 12.sp)
        content()
    }
}

/** A button that opens a menu of choices; the current one is shown. */
@Composable
private fun <T> Choice(current: String, options: List<Pair<T, String>>, choose: (T) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        OutlinedButton(onClick = { open = true }, modifier = Modifier.fillMaxWidth()) {
            Text(current, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            Text("▾")
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            options.forEach { (value, text) ->
                DropdownMenuItem(text = { Text(text) }, onClick = { open = false; choose(value) })
            }
        }
    }
}

/**
 * One folder, project or tag (ADR 0037 §3, §6): name, place, and for a
 * project its kind, status, workflow and review; delete with the server's
 * preview first (ADR 0036).
 */
@Composable
private fun PlaceScreen(id: String, tick: Int, changed: () -> Unit, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var detail by remember { mutableStateOf<PlaceDetail?>(null) }
    var targets by remember { mutableStateOf<List<PlaceRow>>(emptyList()) }
    var workflows by remember { mutableStateOf<List<PlaceRow>>(emptyList()) }
    var scopes by remember { mutableStateOf<List<ScopeRow>>(emptyList()) }
    var name by remember(id) { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var deleting by remember(id) { mutableStateOf(false) }
    LaunchedEffect(tick, id) {
        val found = withContext(Dispatchers.IO) {
            val samu = Store.get(context)
            targets = runCatching { samu.placeTargets(id) }.getOrDefault(emptyList())
            workflows = runCatching { samu.workflows() }.getOrDefault(emptyList())
            scopes = runCatching { samu.scopes() }.getOrDefault(emptyList())
            runCatching { samu.placeDetail(id) }.getOrNull()
        }
        detail = found
        if (found == null) back() else if (name == null) name = found.title
    }
    val d = detail ?: return
    fun act(action: Samu.() -> Unit) {
        scope.launch {
            error = withContext(Dispatchers.IO) { runCatching { Store.get(context).action() }.exceptionOrNull()?.message }
            changed()
        }
    }
    val prefix = if (d.kind == "tag") tr("unter ") else tr("in ")
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Zurück")) } }
        item {
            Text(kindWord(d.kind).uppercase(), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.7.sp)
            Field("Name") {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(name ?: d.title, { name = it }, singleLine = true, modifier = Modifier.weight(1f))
                    val typed = name?.trim().orEmpty()
                    if (typed.isNotEmpty() && typed != d.title) {
                        TextButton(onClick = { act { renamePlace(id, typed) } }) { Text(tr("Sichern")) }
                    }
                }
            }
            Field(tr("Ort")) {
                val options: List<Pair<String?, String>> =
                    listOf<Pair<String?, String>>(null to tr("Oben")) +
                        targets.map { it.id to (" ".repeat(it.depth.toInt()) + prefix + it.title) }
                Choice(d.placeTitle?.let { prefix + it } ?: tr("Oben"), options) { target -> act { movePlace(id, target) } }
            }
            // ADR 0094: „Nur im Scope:“ — shown in that scope alone, whatever
            // its mode; hidden in every other one and without a scope.
            Field(tr("Nur im Scope")) {
                val what = when (d.kind) {
                    "tag" -> tr("Aufgaben mit diesem Tag erscheinen")
                    "folder" -> tr("Was in diesem Ordner liegt, erscheint")
                    else -> tr("Dieses Projekt erscheint")
                }
                val missing = d.ownScope != null && d.ownScopeTitle == null
                val options: List<Pair<String?, String>> =
                    listOf<Pair<String?, String>>(null to tr("– überall –")) + scopes.map { it.id to it.title }
                val current = d.ownScopeTitle ?: if (missing) tr("Scope fehlt") else tr("– überall –")
                Choice(current, options) { scopeId -> act { setOwnScope(id, scopeId) } }
                Text(
                    when {
                        d.ownScopeTitle != null ->
                            tr("{what} nur im Scope „{ownScopeTitle}“ – dort auch ohne Häkchen –, in keinem anderen und auch ohne Scope nicht.", "what" to what, "ownScopeTitle" to d.ownScopeTitle)
                        missing -> tr("Den gewählten Scope gibt es nicht mehr; darum erscheint es überall.")
                        else -> tr("{what} überall.", "what" to what)
                    },
                    color = t.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 4.dp),
                )
            }
            if (d.kind != "tag") {
                // An emoji for the list of projects and its title (ADR 0059 §5).
                Field("Icon") { IconPicker(d.icon) { icon -> act { setPlaceIcon(id, icon) } } }
            }
            if (d.kind == "project") {
                Column(Modifier.padding(top = 10.dp)) { PropertiesPart(id, tick) { action -> act(action) } }
                Field(tr("Art")) {
                    Row(Modifier.fillMaxWidth().border(1.dp, t.border, RoundedCornerShape(8.dp))) {
                        PROJECT_KIND.forEach { (value, text) ->
                            val on = d.projectKind == value
                            Box(
                                Modifier.weight(1f)
                                    .background(if (on) t.select else t.pane, RoundedCornerShape(8.dp))
                                    .clickable { if (!on) act { setProjectKind(id, value) } }
                                    .padding(vertical = 10.dp),
                                contentAlignment = Alignment.Center,
                            ) {
                                Text(
                                    text, fontSize = 13.sp,
                                    color = if (on) t.accent else t.fg,
                                    fontWeight = if (on) FontWeight.SemiBold else FontWeight.Normal,
                                )
                            }
                        }
                    }
                }
                Field("Status") {
                    Choice(STATUS.firstOrNull { it.first == d.status }?.second ?: "?", STATUS) { s -> act { setProjectStatus(id, s) } }
                }
                Field(tr("Ablauf")) {
                    val options: List<Pair<String?, String>> =
                        listOf<Pair<String?, String>>(null to tr("Vorgabe")) + workflows.map { it.id to it.title }
                    Choice(workflows.firstOrNull { it.id == d.workflow }?.title ?: tr("Vorgabe"), options) { w ->
                        act { setProjectWorkflow(id, w) }
                    }
                }
                Field("Review") {
                    Choice(REVIEW.firstOrNull { it.first == d.review }?.second ?: tr("Keine"), REVIEW) { r -> act { setProjectReview(id, r) } }
                }
                Text(
                    if (d.openTasks == 1u) tr("1 offene Aufgabe") else tr("{openTasks} offene Aufgaben", "openTasks" to d.openTasks),
                    color = t.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 10.dp),
                )
            }
            error?.let { Text(it, color = t.overdue, fontSize = 13.sp, modifier = Modifier.padding(top = 8.dp)) }
            if (deleting) {
                DeletePanel(d, changed, { deleting = false }, back)
            } else {
                TextButton(onClick = { deleting = true }, modifier = Modifier.padding(top = 16.dp)) {
                    Text(tr("{n} löschen …", "n" to kindWord(d.kind)), color = t.overdue)
                }
            }
        }
        item { Spacer(Modifier.heightIn(min = 96.dp)) }
    }
}

/** Delete online, after the server's preview (ADR 0036 §1, §7). */
@Composable
private fun DeletePanel(d: PlaceDetail, changed: () -> Unit, cancel: () -> Unit, gone: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    val isProject = d.kind == "project"
    var fate by remember { mutableStateOf("inbox") }
    var target by remember { mutableStateOf<PlaceRow?>(null) }
    var preview by remember { mutableStateOf<DeleteCount?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var projects by remember { mutableStateOf<List<PlaceRow>>(emptyList()) }
    val ready = !isProject || fate != "project" || target != null
    LaunchedEffect(Unit) {
        projects = withContext(Dispatchers.IO) {
            runCatching { Store.get(context).places(false) }.getOrDefault(emptyList())
        }.filter { it.kind == "project" && it.id != d.id }
    }
    LaunchedEffect(fate, target) {
        preview = null
        error = null
        if (!ready) return@LaunchedEffect
        withContext(Dispatchers.IO) {
            try {
                preview = Store.get(context).deletePlace(d.id, if (isProject) fate else null, target?.id, true)
            } catch (e: Exception) {
                error = e.message ?: e.toString()
            }
        }
    }
    Column(
        Modifier.fillMaxWidth().padding(top = 16.dp)
            .border(1.dp, t.overdue, RoundedCornerShape(8.dp))
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(tr("{n} löschen", "n" to kindWord(d.kind)), color = t.fg, fontWeight = FontWeight.SemiBold)
        if (isProject) {
            Text(tr("Die Aufgaben des Projekts"), color = t.muted, fontSize = 12.sp)
            listOf("inbox" to tr("in die Inbox"), "project" to tr("in ein anderes Projekt"), "delete" to tr("mitlöschen")).forEach { (value, text) ->
                Row(Modifier.fillMaxWidth().clickable { fate = value }, verticalAlignment = Alignment.CenterVertically) {
                    RadioButton(selected = fate == value, onClick = { fate = value })
                    Text(text, color = t.fg, fontSize = 14.sp)
                }
            }
            if (fate == "project") {
                Choice(target?.title ?: tr("Projekt wählen …"), projects.map { it to it.title }) { target = it }
            }
        }
        val counts = preview
        when {
            error != null -> Text(error!!, color = t.overdue, fontSize = 13.sp)
            counts != null -> Text(countsText(d.kind, fate, counts), color = t.muted, fontSize = 13.sp)
            ready -> Text("…", color = t.muted, fontSize = 13.sp)
        }
        Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
            OutlinedButton(onClick = cancel) { Text(tr("Abbrechen")) }
            Button(
                enabled = preview != null && error == null,
                colors = ButtonDefaults.buttonColors(containerColor = t.overdue),
                onClick = {
                    scope.launch {
                        val failed = withContext(Dispatchers.IO) {
                            runCatching {
                                Store.get(context).deletePlace(d.id, if (isProject) fate else null, target?.id, false)
                            }.exceptionOrNull()
                        }
                        if (failed != null) {
                            error = failed.message ?: failed.toString()
                        } else {
                            changed()
                            gone()
                        }
                    }
                },
            ) { Text(tr("Löschen")) }
        }
    }
}

private fun countsText(kind: String, fate: String, c: DeleteCount): String = when (kind) {
    "project" -> {
        val n = if (fate == "delete") c.deleted - 1u else c.moved
        when {
            n == 0uL -> tr("Das Projekt hat keine Aufgaben.")
            fate == "delete" -> if (n == 1uL) tr("1 Aufgabe wird mitgelöscht.") else tr("{n} Aufgaben werden mitgelöscht.", "n" to n)
            else -> if (n == 1uL) tr("1 Aufgabe wandert.") else tr("{n} Aufgaben wandern.", "n" to n)
        }
    }
    "folder" -> if (c.moved > 0u) tr("{moved} Projekte und Ordner rücken eine Ebene auf.", "moved" to c.moved) else tr("Der Ordner ist leer.")
    else -> if (c.untagged > 0u) tr("Der Tag wird von {untagged} Aufgaben, Projekten und Scopes genommen.", "untagged" to c.untagged) else tr("Niemand trägt den Tag.")
}
