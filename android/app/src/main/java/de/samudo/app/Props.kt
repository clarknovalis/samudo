// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import de.samudo.app.mobile.PropertyLinkRow
import de.samudo.app.mobile.PropertyRow
import de.samudo.app.mobile.Samu
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Fainter than muted: inherited and system properties. */
private val Tokens.faint get() = muted.copy(alpha = 0.6f)

/** What the editor sheet is open for: a key (new when blank) and its value. */
private data class Editing(val key: String, val value: String, val fresh: Boolean)

/**
 * The properties of a task or project as a front matter under the title
 * (ADR 0054 §2): folded to a line, unfolded one `key  value` per line; a
 * value with a link opens it, a tap elsewhere edits; inherited ones faint
 * with where they come from. Folded or not is this phone's.
 */
@Composable
fun PropertiesPart(id: String, tick: Int, act: (Samu.() -> Unit) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val uri = LocalUriHandler.current
    val scope = rememberCoroutineScope()
    var rows by remember { mutableStateOf<List<PropertyRow>>(emptyList()) }
    var keys by remember { mutableStateOf<List<String>>(emptyList()) }
    var open by remember { mutableStateOf(true) }
    var editing by remember { mutableStateOf<Editing?>(null) }
    LaunchedEffect(tick, id) {
        withContext(Dispatchers.IO) {
            val samu = Store.get(context)
            rows = runCatching { samu.properties(id) }.getOrDefault(emptyList())
            keys = runCatching { samu.propertyKeys() }.getOrDefault(emptyList())
            open = runCatching { samu.propertiesOpen() }.getOrDefault(true)
        }
    }
    editing?.let { e ->
        PropertyEditor(e, keys, close = { editing = null }) { key, value ->
            editing = null
            act { setProperty(id, key, value) }
        }
    }
    if (rows.isEmpty()) {
        Text(
            tr("+ Property"), color = t.faint, fontSize = 12.sp,
            modifier = Modifier.clickable { editing = Editing("", "", true) }.padding(vertical = 2.dp),
        )
        return
    }
    val names = rows.filter { !it.system }.map { it.key }
    val summary = "${rows.size} ${if (rows.size == 1) "Property" else "Properties"}" +
        if (names.isEmpty()) "" else " · " + names.take(3).joinToString(", ") + if (names.size > 3) " …" else ""
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(
            (if (open) "▾  " else "▸  ") + summary,
            color = t.muted, fontSize = 12.sp,
            modifier = Modifier.clickable {
                open = !open
                val now = open
                scope.launch { withContext(Dispatchers.IO) { runCatching { Store.get(context).setPropertiesOpen(now) } } }
            }.padding(vertical = 4.dp),
        )
        if (!open) return@Column
        Column(Modifier.padding(start = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            rows.forEach { r ->
                val faint = !r.own || r.system
                Row(verticalAlignment = Alignment.Top) {
                    Text(
                        r.key, fontFamily = FontFamily.Monospace, fontSize = 12.sp,
                        color = if (faint) t.faint else t.muted,
                        modifier = Modifier.width(96.dp).padding(top = 1.dp),
                    )
                    Column(Modifier.weight(1f)) {
                        val link = r.link
                        Text(
                            r.value, fontSize = 14.sp,
                            color = when {
                                link != null -> t.accent
                                faint -> t.faint
                                else -> t.fg
                            },
                            modifier = Modifier.clickable {
                                if (link != null) runCatching { uri.openUri(link) }
                                else editing = Editing(r.key, if (r.own) r.value else "", false)
                            },
                        )
                        r.from?.let { Text(tr("von „{it}“", "it" to it), color = t.faint, fontSize = 11.sp) }
                    }
                    Text(
                        "✎", color = t.faint, fontSize = 14.sp,
                        modifier = Modifier.clickable { editing = Editing(r.key, if (r.own) r.value else "", false) }
                            .padding(horizontal = 8.dp),
                    )
                }
            }
            Text(
                tr("+ Property"), color = t.faint, fontSize = 12.sp,
                modifier = Modifier.clickable { editing = Editing("", "", true) }.padding(vertical = 2.dp),
            )
        }
    }
}

/** Key (for a new one) and value; an empty value removes the property. */
@Composable
private fun PropertyEditor(e: Editing, keys: List<String>, close: () -> Unit, save: (String, String?) -> Unit) {
    val t = LocalTokens.current
    var key by remember(e) { mutableStateOf(e.key) }
    var value by remember(e) { mutableStateOf(e.value) }
    AlertDialog(
        onDismissRequest = close,
        title = { Text(if (e.fresh) tr("Neue Property") else e.key, fontFamily = if (e.fresh) null else FontFamily.Monospace) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (e.fresh) {
                    OutlinedTextField(key, { key = it.lowercase().trim() }, singleLine = true, label = { Text(tr("Schlüssel")) })
                    val hits = keys.filter { it.startsWith(key) && it != key }.take(5)
                    if (hits.isNotEmpty()) {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            hits.forEach { k ->
                                Text(k, color = t.accent, fontFamily = FontFamily.Monospace, fontSize = 13.sp,
                                    modifier = Modifier.clickable { key = k }.padding(4.dp))
                            }
                        }
                    }
                }
                OutlinedTextField(value, { value = it }, singleLine = true, label = { Text(tr("Wert")) })
                if (!e.fresh) Text(tr("Leer entfernt die Property."), color = t.muted, fontSize = 12.sp)
            }
        },
        confirmButton = {
            TextButton(enabled = key.isNotBlank() && (!e.fresh || value.isNotBlank()), onClick = {
                save(key, value.ifBlank { null })
            }) { Text(tr("Sichern")) }
        },
        dismissButton = { TextButton(onClick = close) { Text(tr("Abbrechen")) } },
    )
}

/**
 * Settings › Properties (ADR 0054 §3): the templates there are, each with
 * its field, "Sichern" once changed and × to remove; one row below adds
 * one. A mistake shows under its row. The account's, on every device.
 */
@Composable
fun PropertiesSection(tick: Int, changed: () -> Unit, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var links by remember { mutableStateOf<List<PropertyLinkRow>>(emptyList()) }
    var keys by remember { mutableStateOf<List<String>>(emptyList()) }
    var typed by remember { mutableStateOf<Map<String, String>>(emptyMap()) }
    var errors by remember { mutableStateOf<Map<String, String>>(emptyMap()) }
    var newKey by remember { mutableStateOf("") }
    var newTemplate by remember { mutableStateOf("") }
    LaunchedEffect(tick) {
        withContext(Dispatchers.IO) {
            val samu = Store.get(context)
            links = runCatching { samu.propertyLinks() }.getOrDefault(emptyList())
            keys = runCatching { samu.propertyKeys() }.getOrDefault(emptyList())
        }
    }
    fun set(row: String, key: String, template: String?, done: () -> Unit = {}) {
        scope.launch {
            val error = withContext(Dispatchers.IO) {
                runCatching { Store.get(context).setPropertyLink(key, template) }.exceptionOrNull()?.message
            }
            errors = if (error == null) errors - row else errors + (row to error)
            if (error == null) done()
            changed()
        }
    }
    val unused = keys.filter { k -> links.none { it.key == k } }
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Einstellungen")) } }
        item {
            Text(
                tr("Eine Vorlage macht aus dem Wert einer Property einen Link: mit …/browse/{value} wird ") +
                    tr("jira HAUS-12 zu einem Link auf …/browse/HAUS-12. Ein Wert, der selbst eine Adresse ist ") +
                    tr("(https:, catastr:, mailto: …), braucht keine. Gilt für das Konto, auf allen Geräten."),
                color = t.muted, fontSize = 13.sp,
            )
        }
        if (links.isEmpty()) item { Text(tr("Noch keine Vorlage."), color = t.muted, fontSize = 13.sp) }
        items(links, key = { it.key }) { link ->
            val text = typed[link.key] ?: link.template
            Column {
                Text(link.key, fontFamily = FontFamily.Monospace, fontSize = 13.sp, color = t.fg)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        text, { typed = typed + (link.key to it) }, singleLine = true,
                        modifier = Modifier.weight(1f),
                    )
                    if (text != link.template) {
                        TextButton(onClick = { set(link.key, link.key, text.ifBlank { null }) { typed = typed - link.key } }) { Text(tr("Sichern")) }
                    }
                    TextButton(onClick = { set(link.key, link.key, null) { typed = typed - link.key } }) { Text("×", fontSize = 18.sp) }
                }
                errors[link.key]?.let { Text(it, color = t.overdue, fontSize = 12.sp) }
            }
        }
        item {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(tr("VORLAGE HINZUFÜGEN"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
                OutlinedTextField(
                    newKey, { newKey = it.lowercase().trim() }, singleLine = true,
                    label = { Text(tr("Schlüssel")) }, modifier = Modifier.fillMaxWidth(),
                )
                if (unused.isNotEmpty()) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        unused.filter { it.startsWith(newKey) && it != newKey }.take(5).forEach { k ->
                            Text(k, color = t.accent, fontFamily = FontFamily.Monospace, fontSize = 13.sp,
                                modifier = Modifier.clickable { newKey = k }.padding(4.dp))
                        }
                    }
                }
                OutlinedTextField(
                    newTemplate, { newTemplate = it }, singleLine = true,
                    label = { Text(tr("Vorlage mit {value}")) }, modifier = Modifier.fillMaxWidth(),
                )
                TextButton(
                    enabled = newKey.isNotBlank() && newTemplate.isNotBlank(),
                    onClick = {
                        if (links.any { it.key == newKey }) {
                            errors = errors + ("+" to tr("Für {newKey} gibt es schon eine Vorlage; oben ändern.", "newKey" to newKey))
                        } else {
                            set("+", newKey, newTemplate) { newKey = ""; newTemplate = "" }
                        }
                    },
                ) { Text(tr("Hinzufügen")) }
                errors["+"]?.let { Text(it, color = t.overdue, fontSize = 12.sp) }
            }
        }
    }
}
