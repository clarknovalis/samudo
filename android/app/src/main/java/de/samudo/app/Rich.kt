// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.content.Intent
import android.net.Uri
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TriStateCheckbox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * A note or comment rendered (ADR 0058 §7). samu-mobile reads the format
 * off the text and renders it with samu-core, task links found in the
 * cache; here the tree becomes Compose. Nothing in a note runs: only web
 * links and found task links are followed, a node not known is left out.
 *
 * [onText] makes checkboxes switch: it gets the text with the box changed.
 * [open] opens a task by its id.
 */
@Composable
fun RichText(text: String, onText: ((String) -> Unit)?, open: (String) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var tree by remember(text) { mutableStateOf<JSONObject?>(null) }
    LaunchedEffect(text) {
        tree = withContext(Dispatchers.IO) {
            runCatching { JSONObject(Store.get(context).renderNote(text)) }.getOrNull()
        }
    }
    val doc = tree
    if (doc == null) {
        Text(text, color = t.fg, fontSize = 14.sp)
        return
    }
    val toggle: ((JSONObject) -> Unit)? = onText?.let { save ->
        { box ->
            val next = Store.get(context).toggleCheckbox(text, box.getLong("start").toULong(), box.getLong("end").toULong())
            if (next != null) save(next)
        }
    }
    val ctx = RenderContext(
        t,
        toggle,
        open,
        web = { url -> runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) } },
        // Another app of the phone takes it (ADR 0075 §5); none: say so.
        app = { uri ->
            runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(uri))) }.onFailure {
                Toast.makeText(context, tr("Keine App öffnet {n}:", "n" to uri.substringBefore(':')), Toast.LENGTH_SHORT).show()
            }
        },
    )
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Blocks(doc.optJSONArray("blocks") ?: JSONArray(), ctx)
    }
}

private class RenderContext(
    val t: Tokens,
    val toggle: ((JSONObject) -> Unit)?,
    val open: (String) -> Unit,
    val web: (String) -> Unit,
    val app: (String) -> Unit,
)

private fun JSONArray.objects(): List<JSONObject> = (0 until length()).mapNotNull { optJSONObject(it) }

@Composable
private fun Blocks(blocks: JSONArray, ctx: RenderContext) {
    blocks.objects().forEach { Block(it, ctx) }
}

@Composable
private fun Block(b: JSONObject, ctx: RenderContext) {
    val t = ctx.t
    when (b.optString("type")) {
        "heading" -> {
            val level = b.optInt("level", 1)
            val text = buildAnnotatedString {
                b.optString("keyword").takeIf { it.isNotEmpty() }?.let {
                    withStyle(SpanStyle(color = if (b.optBoolean("done")) t.ok else t.accent, fontWeight = FontWeight.Bold, fontSize = 12.sp)) { append(it) }
                    append(" ")
                }
                b.optString("priority").takeIf { it.isNotEmpty() }?.let {
                    withStyle(SpanStyle(color = t.muted, fontWeight = FontWeight.Normal)) { append("[#$it] ") }
                }
                inlines(b.optJSONArray("content"), ctx)
                b.optJSONArray("tags")?.let { tags ->
                    for (i in 0 until tags.length()) {
                        withStyle(SpanStyle(color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.Normal)) { append("  :" + tags.optString(i) + ":") }
                    }
                }
            }
            Text(text, color = t.fg, fontSize = if (level <= 1) 17.sp else 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 4.dp))
        }
        "paragraph" -> Text(inline(b.optJSONArray("content"), ctx), color = t.fg, fontSize = 14.sp)
        "verse" -> Text(inline(b.optJSONArray("content"), ctx), color = t.fg, fontSize = 14.sp, fontStyle = FontStyle.Italic)
        "list" -> ListBlock(b, ctx)
        "table" -> TableBlock(b, ctx)
        "code", "html" -> Box(Modifier.fillMaxWidth().background(t.rowline, RoundedCornerShape(6.dp)).horizontalScroll(rememberScrollState()).padding(8.dp)) {
            Text(b.optString("text").trimEnd('\n'), color = t.fg, fontSize = 13.sp, fontFamily = FontFamily.Monospace, softWrap = false)
        }
        "quote" -> Row {
            Box(Modifier.width(3.dp).padding(vertical = 2.dp).background(t.line).size(3.dp, 20.dp))
            Column(Modifier.padding(start = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) { Blocks(b.optJSONArray("blocks") ?: JSONArray(), ctx) }
        }
        "center" -> Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) { Blocks(b.optJSONArray("blocks") ?: JSONArray(), ctx) }
        "keyword" -> Text(buildAnnotatedString {
            withStyle(SpanStyle(color = t.muted)) { append(b.optString("key") + ": ") }
            append(b.optString("value"))
        }, color = t.fg, fontSize = 13.sp)
        "properties", "drawer" -> Drawer(b, ctx)
        "rule" -> Box(Modifier.fillMaxWidth().padding(vertical = 4.dp).background(t.line).size(1.dp, 1.dp))
        "footnote" -> Row {
            Text(b.optString("label") + " ", color = t.muted, fontSize = 12.sp)
            Column { Blocks(b.optJSONArray("blocks") ?: JSONArray(), ctx) }
        }
        else -> {}
    }
}

/** A drawer folded; a tap opens it. */
@Composable
private fun Drawer(b: JSONObject, ctx: RenderContext) {
    val t = ctx.t
    var open by remember { mutableStateOf(false) }
    val name = if (b.optString("type") == "properties") "Properties" else b.optString("name")
    Column {
        Text((if (open) "▾ " else "▸ ") + name, color = t.muted, fontSize = 12.sp, modifier = Modifier.clickable { open = !open })
        if (open) {
            val body = if (b.optString("type") == "properties") {
                (b.optJSONArray("entries") ?: JSONArray()).objects().joinToString("\n") { it.optString("key") + ": " + it.optString("value") }
            } else b.optString("text")
            Text(body, color = t.muted, fontSize = 12.sp, fontFamily = FontFamily.Monospace)
        }
    }
}

@Composable
private fun ListBlock(b: JSONObject, ctx: RenderContext) {
    val t = ctx.t
    val ordered = b.optBoolean("ordered")
    var number = b.optLong("start", 1)
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        (b.optJSONArray("items") ?: JSONArray()).objects().forEach { item ->
            val checked = if (item.has("checked")) item.optBoolean("checked") else null
            val box = item.optJSONObject("box")
            Row(verticalAlignment = Alignment.Top) {
                when {
                    checked != null -> {
                        val toggle = ctx.toggle?.takeIf { box != null }
                        val colors = CheckboxDefaults.colors(checkedColor = t.accent)
                        val modifier = Modifier.size(32.dp)
                        if (item.optBoolean("partial")) {
                            TriStateCheckbox(ToggleableState.Indeterminate, onClick = toggle?.let { { it(box!!) } }, modifier = modifier, colors = colors)
                        } else {
                            Checkbox(checked, onCheckedChange = toggle?.let { { _ -> it(box!!) } }, modifier = modifier, colors = colors)
                        }
                    }
                    ordered -> Text("${number}.", color = t.muted, fontSize = 14.sp, modifier = Modifier.width(24.dp))
                    else -> Text("•", color = t.muted, fontSize = 14.sp, modifier = Modifier.width(16.dp).padding(start = 4.dp))
                }
                Column(Modifier.weight(1f).padding(top = if (checked != null) 6.dp else 0.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    item.optJSONArray("term")?.let { term ->
                        Text(inline(term, ctx), color = t.fg, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
                    }
                    val inner = item.optJSONArray("blocks") ?: JSONArray()
                    if (checked == true) {
                        // Done items read as done: muted and struck through.
                        inner.objects().forEach { child ->
                            if (child.optString("type") == "paragraph") {
                                Text(inline(child.optJSONArray("content"), ctx), color = t.muted, fontSize = 14.sp, textDecoration = TextDecoration.LineThrough)
                            } else Block(child, ctx)
                        }
                    } else Blocks(inner, ctx)
                }
            }
            number += 1
        }
    }
}

@Composable
private fun TableBlock(b: JSONObject, ctx: RenderContext) {
    val t = ctx.t
    Column(Modifier.horizontalScroll(rememberScrollState()).border(1.dp, t.line)) {
        (b.optJSONArray("rows") ?: JSONArray()).objects().forEach { row ->
            val header = row.optBoolean("header")
            Row(if (header) Modifier.background(t.rowline) else Modifier) {
                val cells = row.optJSONArray("cells") ?: JSONArray()
                for (i in 0 until cells.length()) {
                    Text(
                        inline(cells.optJSONArray(i), ctx), color = t.fg, fontSize = 13.sp,
                        fontWeight = if (header) FontWeight.SemiBold else FontWeight.Normal,
                        modifier = Modifier.border(0.5.dp, t.line).padding(horizontal = 8.dp, vertical = 4.dp).width(110.dp),
                    )
                }
            }
        }
    }
}

private fun inline(content: JSONArray?, ctx: RenderContext): AnnotatedString = buildAnnotatedString { inlines(content, ctx) }

private fun AnnotatedString.Builder.inlines(content: JSONArray?, ctx: RenderContext) {
    content?.objects()?.forEach { inline(it, ctx) }
}

private fun AnnotatedString.Builder.inline(i: JSONObject, ctx: RenderContext) {
    val t = ctx.t
    val content = i.optJSONArray("content")
    when (i.optString("type")) {
        "text" -> append(i.optString("text"))
        "strong" -> withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { inlines(content, ctx) }
        "emphasis" -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { inlines(content, ctx) }
        "underline" -> withStyle(SpanStyle(textDecoration = TextDecoration.Underline)) { inlines(content, ctx) }
        "strike" -> withStyle(SpanStyle(textDecoration = TextDecoration.LineThrough)) { inlines(content, ctx) }
        "code", "html" -> withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = t.rowline)) { append(i.optString("text")) }
        "timestamp" -> withStyle(SpanStyle(color = t.muted)) { append(i.optString("text")) }
        "footnote_ref" -> withStyle(SpanStyle(color = t.muted, fontSize = 11.sp)) { append("[" + i.optString("label") + "]") }
        "break" -> append("\n")
        "link", "image" -> {
            val target = i.optString("target")
            val label: AnnotatedString.Builder.() -> Unit = {
                if (i.optString("type") == "image") append(i.optString("alt").ifEmpty { target })
                else if (content != null && content.length() > 0) inlines(content, ctx) else append(target)
            }
            // A task or project the server found (ADR 0077 §4); `task` alone
            // from a server before it. Folders and tags stay text for now.
            val linked = i.optJSONObject("object")
            val task = when {
                linked != null && linked.optString("kind") in setOf("task", "project") -> linked.optString("id")
                linked != null -> null
                else -> i.optString("task")
            }?.takeIf { it.isNotEmpty() }
            val styles = TextLinkStyles(SpanStyle(color = t.accent, textDecoration = TextDecoration.Underline))
            when {
                i.optString("class") == "web" -> withLink(LinkAnnotation.Clickable(target, styles) { ctx.web(target) }) { label() }
                i.optString("class") == "task" && task != null -> withLink(LinkAnnotation.Clickable(task, styles) { ctx.open(task) }) { label() }
                i.optString("class") == "app" -> withLink(LinkAnnotation.Clickable(target, styles) { ctx.app(target) }) { label() }
                // Shown, never followed (ADR 0058 §5).
                else -> withStyle(SpanStyle(color = t.muted, textDecoration = TextDecoration.Underline)) { label() }
            }
        }
        else -> {}
    }
}

/**
 * The note of a task (ADR 0058 §7): rendered with a quiet pencil that
 * switches to the field; an empty note is the field at once. Leaving the
 * field or "Fertig" saves and shows it rendered again.
 */
@Composable
fun NoteField(note: String, save: (String?) -> Unit, open: (String) -> Unit, rendered: Boolean = false) {
    val t = LocalTokens.current
    var editing by remember(note) { mutableStateOf(false) }
    // A scope that wants the note rendered (ADR 0072 §2): empty, a quiet
    // line instead of the field.
    if (rendered && note.isBlank() && !editing) {
        Text(
            tr("Notiz schreiben"), color = t.muted, fontSize = 14.sp,
            modifier = Modifier.clickable { editing = true }.padding(vertical = 6.dp),
        )
        return
    }
    if (note.isBlank() || editing) {
        NoteEditor(note, focus = editing) { text ->
            if (text != note) save(text.ifBlank { null })
            editing = false
        }
        return
    }
    Row(verticalAlignment = Alignment.Top) {
        Box(Modifier.weight(1f)) { RichText(note, { next -> save(next) }, open) }
        Text(
            "✎", color = t.muted, fontSize = 16.sp,
            modifier = Modifier.clickable { editing = true }.padding(start = 8.dp, end = 4.dp, bottom = 8.dp),
        )
    }
}

@Composable
private fun NoteEditor(value: String, focus: Boolean, done: (String) -> Unit) {
    var text by remember(value) { mutableStateOf(value) }
    var focused by remember { mutableStateOf(false) }
    val requester = remember { androidx.compose.ui.focus.FocusRequester() }
    LaunchedEffect(focus) { if (focus) runCatching { requester.requestFocus() } }
    Column {
        androidx.compose.material3.OutlinedTextField(
            text, { text = it },
            placeholder = { Text(tr("Notiz – Markdown oder Org")) },
            textStyle = androidx.compose.ui.text.TextStyle(fontSize = 14.sp, color = LocalTokens.current.fg),
            modifier = Modifier.fillMaxWidth().focusRequester(requester).onFocusChanged { state ->
                if (focused && !state.isFocused) done(text)
                focused = state.isFocused
            },
        )
        if (focused) androidx.compose.material3.TextButton(onClick = { done(text) }) { Text(tr("Fertig")) }
    }
}
