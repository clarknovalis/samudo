// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.media.ExifInterface
import android.media.MediaPlayer
import android.media.MediaRecorder
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import de.samudo.app.mobile.AttachmentRow
import de.samudo.app.mobile.CommentRow
import de.samudo.app.mobile.Samu
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/** A file taken or picked, not yet handed to samu-mobile. */
private data class Draft(val path: String, val name: String, val mediaType: String)

private const val AUTHORITY = "de.samudo.app.files"

/** Where taken and picked files wait until they are sent; samu-mobile moves them away. */
private fun drafts(context: Context): File = context.cacheDir.resolve("notes").also { it.mkdirs() }

private fun stamp(): String = java.time.LocalDateTime.now().toString().take(19).replace(':', '-')

/**
 * Comments and attachments of a task (ADR 0039 §6): the files on the task,
 * then the comments oldest first, then one field for text and files. With
 * text it becomes a comment with the files on it; without, the files go on
 * the task, as in the web UI.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun NotesPart(
    owner: String,
    tick: Int,
    act: (Samu.() -> Unit) -> Unit,
    open: (String) -> Unit = {},
    showComments: Boolean = true,
    showFiles: Boolean = true,
) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var comments by remember { mutableStateOf<List<CommentRow>>(emptyList()) }
    var files by remember { mutableStateOf<List<AttachmentRow>>(emptyList()) }
    LaunchedEffect(tick, owner) {
        withContext(Dispatchers.IO) {
            val samu = Store.get(context)
            comments = runCatching { samu.comments(owner) }.getOrDefault(emptyList())
            files = runCatching { samu.attachments(owner) }.getOrDefault(emptyList())
        }
    }

    if (!showComments && !showFiles) return
    if (showFiles && files.isNotEmpty()) {
        Text(tr("ANHÄNGE"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        files.forEach { AttachmentView(it, act) }
    }
    if (showComments) {
        Text(tr("KOMMENTARE"), color = t.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        if (comments.isEmpty()) Text(tr("Noch keine."), color = t.muted, fontSize = 13.sp)
        comments.forEach { CommentView(it, act, open) }
    }
    Composer(owner, act)
}

@Composable
private fun CommentView(c: CommentRow, act: (Samu.() -> Unit) -> Unit, open: (String) -> Unit) {
    val t = LocalTokens.current
    var editing by remember(c.id) { mutableStateOf(false) }
    var text by remember(c.id, c.body) { mutableStateOf(c.body) }
    var asking by remember(c.id) { mutableStateOf(false) }
    Column(
        Modifier.fillMaxWidth().background(t.pane, RoundedCornerShape(8.dp)).padding(10.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(c.author ?: "–", color = t.fg, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
            if (c.agent) {
                Text(
                    "Agent", color = t.accent, fontSize = 11.sp,
                    modifier = Modifier.background(t.select, RoundedCornerShape(4.dp)).padding(horizontal = 5.dp, vertical = 1.dp),
                )
            }
            Text(
                Words.instant(c.createdMs) + (if (c.edited) tr(" · bearbeitet") else "") + (if (c.pending) tr(" · im Ausgang") else ""),
                color = if (c.pending) t.wait else t.muted, fontSize = 12.sp,
            )
        }
        if (editing) {
            OutlinedTextField(text, { text = it }, modifier = Modifier.fillMaxWidth())
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = { act { editComment(c.id, text) }; editing = false }, enabled = text.isNotBlank()) { Text(tr("Sichern")) }
                TextButton(onClick = { text = c.body; editing = false }) { Text(tr("Abbrechen")) }
            }
        } else {
            // Rendered (ADR 0058 §7); checkboxes only where it may be changed.
            RichText(c.body, if (c.changeable) { next -> act { editComment(c.id, next) } } else null, open)
        }
        c.attachments.forEach { AttachmentView(it, act) }
        if (c.changeable && !editing) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (asking) {
                    Text(tr("Kommentar löschen?"), color = t.fg, fontSize = 13.sp)
                    TextButton(onClick = { act { deleteNote(c.id) }; asking = false }) { Text(tr("Ja")) }
                    TextButton(onClick = { asking = false }) { Text(tr("Nein")) }
                } else {
                    TextButton(onClick = { editing = true }) { Text(tr("Bearbeiten")) }
                    TextButton(onClick = { asking = true }) { Text(tr("Löschen")) }
                }
            }
        }
    }
}

/** A picture as a preview, a recording with play and stop, anything else to open. */
@Composable
private fun AttachmentView(a: AttachmentRow, act: (Samu.() -> Unit) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var path by remember(a.hash) { mutableStateOf(a.local) }
    var failed by remember(a.hash) { mutableStateOf<String?>(null) }
    var picture by remember(a.hash) { mutableStateOf<Bitmap?>(null) }
    var asking by remember(a.id) { mutableStateOf(false) }
    val image = a.mediaType.startsWith("image/")
    val sound = a.mediaType.startsWith("audio/")
    LaunchedEffect(a.hash) {
        if (image) {
            withContext(Dispatchers.IO) {
                runCatching { path ?: Store.get(context).fetchBlob(a.hash) }
                    .onSuccess { p -> path = p; if (image) picture = preview(p) }
                    .onFailure { failed = it.message }
            }
        }
    }
    val fetchThen: (String.() -> Unit) -> Unit = { then ->
        path?.then() ?: Thread {
            runCatching { Store.get(context).fetchBlob(a.hash) }
                .onSuccess { p -> path = p; p.then() }
                .onFailure { failed = it.message }
        }.start()
    }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        picture?.let {
            Image(
                it.asImageBitmap(), a.name,
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxWidth().heightIn(max = 320.dp).clip(RoundedCornerShape(6.dp))
                    .clickable { fetchThen { open(context, this, a.mediaType) } },
            )
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                (if (sound) "♪ " else if (image) "" else "▤ ") + a.name + " · " + size(a.size) +
                    (if (a.agent) " · Agent" else "") + (if (a.pending) tr(" · im Ausgang") else ""),
                color = if (a.pending) t.wait else t.muted, fontSize = 12.sp, modifier = Modifier.weight(1f),
            )
            when {
                sound -> Player(a, fetchThen)
                !image -> TextButton(onClick = { fetchThen { open(context, this, a.mediaType) } }) { Text(tr("Öffnen")) }
            }
            if (a.changeable) {
                if (asking) {
                    TextButton(onClick = { act { deleteNote(a.id) }; asking = false }) { Text(tr("Weg")) }
                    TextButton(onClick = { asking = false }) { Text(tr("Nein")) }
                } else {
                    TextButton(onClick = { asking = true }) { Text(tr("Entfernen")) }
                }
            }
        }
        failed?.let { Text(tr("Nicht geladen: {it}", "it" to it), color = t.overdue, fontSize = 12.sp) }
    }
}

/** One recording at a time per row; stops when the row leaves. */
@Composable
private fun Player(a: AttachmentRow, fetchThen: (String.() -> Unit) -> Unit) {
    var player by remember(a.hash) { mutableStateOf<MediaPlayer?>(null) }
    DisposableEffect(a.hash) { onDispose { player?.release(); player = null } }
    if (player == null) {
        TextButton(onClick = {
            fetchThen {
                val p = MediaPlayer()
                runCatching {
                    p.setDataSource(this)
                    p.setOnCompletionListener { it.release(); player = null }
                    p.prepare()
                    p.start()
                    player = p
                }.onFailure { p.release() }
            }
        }) { Text(tr("▶ Abspielen")) }
    } else {
        TextButton(onClick = { player?.release(); player = null }) { Text(tr("■ Stopp")) }
    }
}

/** The text field and the ways to add files. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Composer(owner: String, act: (Samu.() -> Unit) -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var text by remember(owner) { mutableStateOf("") }
    val waiting = remember(owner) { mutableStateListOf<Draft>() }
    var photo by remember { mutableStateOf<File?>(null) }
    var recorder by remember { mutableStateOf<MediaRecorder?>(null) }
    var recording by remember { mutableStateOf<File?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }

    val take = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { ok ->
        val file = photo ?: return@rememberLauncherForActivityResult
        if (ok && file.length() > 0) waiting += Draft(file.path, file.name, "image/jpeg") else file.delete()
        photo = null
    }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        if (uri != null) runCatching { waiting += copied(context, uri) }.onFailure { problem = it.message }
    }
    val start: () -> Unit = {
        val file = drafts(context).resolve("aufnahme-${stamp()}.m4a")
        runCatching { recorder = record(context, file); recording = file }
            .onFailure { problem = tr("Aufnahme geht nicht: {message}", "message" to it.message); file.delete() }
    }
    val askMic = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) start() else problem = tr("Ohne Mikrofon keine Aufnahme.")
    }
    val stop: () -> Unit = {
        val file = recording
        runCatching { recorder?.stop() }
        recorder?.release()
        recorder = null
        recording = null
        if (file != null && file.length() > 0) waiting += Draft(file.path, file.name, "audio/mp4") else file?.delete()
    }
    DisposableEffect(owner) { onDispose { recorder?.release(); recorder = null } }

    OutlinedTextField(
        text, { text = it },
        placeholder = { Text(tr("Kommentar …")) },
        modifier = Modifier.fillMaxWidth(),
    )
    waiting.forEach { d ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(d.name, color = t.fg, fontSize = 13.sp, modifier = Modifier.weight(1f))
            TextButton(onClick = { File(d.path).delete(); waiting -= d }) { Text("✕") }
        }
    }
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton(onClick = {
            val file = drafts(context).resolve("foto-${stamp()}.jpg")
            photo = file
            take.launch(FileProvider.getUriForFile(context, AUTHORITY, file))
        }, enabled = recorder == null) { Text(tr("Foto")) }
        if (recorder == null) {
            OutlinedButton(onClick = {
                if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) start()
                else askMic.launch(Manifest.permission.RECORD_AUDIO)
            }) { Text(tr("● Aufnehmen")) }
        } else {
            Button(onClick = stop) { Text(tr("■ Aufnahme beenden")) }
        }
        OutlinedButton(onClick = { pick.launch("*/*") }, enabled = recorder == null) { Text(tr("Datei")) }
        Button(
            onClick = {
                val body = text.trim()
                val files = waiting.toList()
                act {
                    val target = if (body.isNotEmpty()) addComment(owner, body) else owner
                    files.forEach { attachFile(target, it.path, it.name, it.mediaType) }
                }
                text = ""
                waiting.clear()
                problem = null
            },
            enabled = recorder == null && (text.isNotBlank() || waiting.isNotEmpty()),
        ) { Text(if (text.isBlank() && waiting.isNotEmpty()) tr("Anhängen") else tr("Senden")) }
    }
    problem?.let { Text(it, color = t.overdue, fontSize = 13.sp) }
}

/** AAC in MPEG-4: what every phone records from API 26 and every browser plays. */
private fun record(context: Context, file: File): MediaRecorder {
    val r = if (Build.VERSION.SDK_INT >= 31) MediaRecorder(context) else @Suppress("DEPRECATION") MediaRecorder()
    try {
        r.setAudioSource(MediaRecorder.AudioSource.MIC)
        r.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
        r.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
        r.setAudioEncodingBitRate(64_000)
        r.setAudioSamplingRate(44_100)
        r.setOutputFile(file.path)
        r.prepare()
        r.start()
    } catch (e: Exception) {
        r.release()
        throw e
    }
    return r
}

/** A picked file copied where samu-mobile can take it, with its name and type. */
private fun copied(context: Context, uri: Uri): Draft {
    val resolver = context.contentResolver
    val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
        if (c.moveToFirst()) c.getString(0) else null
    } ?: "datei-${stamp()}"
    val type = resolver.getType(uri) ?: "application/octet-stream"
    val file = drafts(context).resolve("${System.nanoTime()}-${name.replace('/', '_')}")
    resolver.openInputStream(uri).use { input ->
        requireNotNull(input) { tr("die Datei lässt sich nicht lesen") }
        file.outputStream().use { input.copyTo(it) }
    }
    return Draft(file.path, name, type)
}

/** A picture no wider than needed for the screen, turned as the camera held it. */
private fun preview(path: String): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(path, bounds)
    if (bounds.outWidth <= 0) return null
    var sample = 1
    while (bounds.outWidth / (sample * 2) >= 1080) sample *= 2
    val bitmap = BitmapFactory.decodeFile(path, BitmapFactory.Options().apply { inSampleSize = sample }) ?: return null
    val degrees = runCatching {
        when (ExifInterface(path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)) {
            ExifInterface.ORIENTATION_ROTATE_90 -> 90f
            ExifInterface.ORIENTATION_ROTATE_180 -> 180f
            ExifInterface.ORIENTATION_ROTATE_270 -> 270f
            else -> 0f
        }
    }.getOrDefault(0f)
    if (degrees == 0f) return bitmap
    return Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, Matrix().apply { postRotate(degrees) }, true)
}

/** Hands a file to another app to show; the file is named by its hash, so the type goes along. */
private fun open(context: Context, path: String, mediaType: String) {
    val uri = FileProvider.getUriForFile(context, AUTHORITY, File(path))
    val intent = Intent(Intent.ACTION_VIEW).setDataAndType(uri, mediaType)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    runCatching { context.startActivity(Intent.createChooser(intent, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
}

private fun size(bytes: ULong): String {
    val b = bytes.toLong()
    return when {
        b >= 1_048_576 -> "%.1f MB".format(b / 1_048_576.0)
        b >= 1024 -> "${b / 1024} KB"
        else -> "$b B"
    }
}
