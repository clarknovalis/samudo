// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.Manifest
import android.content.ClipData
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.MediaRecorder
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.content.IntentCompat
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.time.LocalDateTime

/**
 * The recorder (ADR 0140 §6 to §8): it records as soon as it opens, and
 * stop makes a voice note in the inbox, whose title and note the server
 * fills from the transcript. It is what the digital assistant key opens
 * when the app holds that role, what the shortcut opens, and where audio
 * shared from another app arrives.
 *
 * It may show over the lock screen and shows nothing else of the app.
 * Leaving it while it records keeps the recording as a voice note.
 */
class VoiceActivity : ComponentActivity() {
    /** What the recorder does when the activity is left. */
    private var whenLeft: () -> Unit = {}

    override fun onStop() {
        super.onStop()
        if (!isChangingConfigurations) whenLeft()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) setShowWhenLocked(true)
        I18n.init(this, Store.get(this))
        Voice.sweep(this)
        val shared = Voice.sharedAudio(intent)
        setContent {
            SamuTheme {
                if (shared != null) SharedAudio(shared) { finish() }
                else Recorder(onLeft = { whenLeft = it }) { finish() }
            }
        }
    }
}

object Voice {
    /** In the cache, under the FileProvider's `notes/`. */
    private fun dir(context: Context): File = context.cacheDir.resolve("notes").also { it.mkdirs() }

    private const val PREFIX = "voice-"
    private const val A_DAY = 24 * 60 * 60 * 1000L

    fun newFile(context: Context): File = dir(context).resolve("$PREFIX${stamp()}.m4a")

    private fun stamp(): String = LocalDateTime.now().toString().take(19).replace(':', '-')

    /** "Sprachnotiz 21.11. 10:05", until the transcript replaces it. */
    fun title(): String {
        val now = LocalDateTime.now()
        return "${tr("Sprachnotiz")} ${now.dayOfMonth}.${now.monthValue}. " +
            "%02d:%02d".format(now.hour, now.minute)
    }

    /**
     * A recording handed to another app stays in the cache until that app
     * has read it; what is older than a day goes.
     */
    fun sweep(context: Context) {
        val old = System.currentTimeMillis() - A_DAY
        dir(context).listFiles { f -> f.name.startsWith(PREFIX) && f.lastModified() < old }
            ?.forEach { it.delete() }
    }

    /** The audio of an `ACTION_SEND`, if the intent is one. */
    fun sharedAudio(intent: Intent?): Uri? {
        if (intent?.action != Intent.ACTION_SEND || intent.type?.startsWith("audio/") != true) return null
        return IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
    }

    /** Files a recording as a voice note; the file moves into the store. */
    suspend fun file(context: Context, file: File, name: String, mediaType: String): Result<String> =
        withContext(Dispatchers.IO) {
            runCatching {
                val id = Store.get(context).voiceNote(title(), file.path, name, mediaType)
                Sync.afterChange(context)
                Widgets.refresh(context)
                redrawApp()
                id
            }
        }

    /** Hands a recording to an app the user picks (§8); samudo keeps nothing. */
    fun handOn(context: Context, file: File) {
        val uri = FileProvider.getUriForFile(context, context.packageName + ".files", file)
        val send = Intent(Intent.ACTION_SEND)
            .setType("audio/mp4")
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        send.clipData = ClipData.newRawUri(null, uri)
        val chooser = Intent.createChooser(send, tr("Aufnahme weitergeben"))
            // Not back to this app.
            .putExtra(Intent.EXTRA_EXCLUDE_COMPONENTS, arrayOf(ComponentName(context, VoiceActivity::class.java)))
        context.startActivity(chooser)
    }

    /** The shortcut on the app's icon (§6). */
    fun shortcut(context: Context) {
        val shortcut = ShortcutInfoCompat.Builder(context, "voice")
            .setShortLabel(tr("Sprachnotiz"))
            .setIcon(IconCompat.createWithResource(context, R.mipmap.ic_launcher))
            .setIntent(Intent(context, VoiceActivity::class.java).setAction(Intent.ACTION_VIEW))
            .build()
        runCatching { ShortcutManagerCompat.setDynamicShortcuts(context, listOf(shortcut)) }
    }

    /** AAC in MPEG-4: what every phone records from API 26 and every browser plays. */
    fun record(context: Context, file: File): MediaRecorder {
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
}

/** Records from the first moment; stop files, discard forgets, hand on shares. */
@Composable
private fun Recorder(onLeft: (() -> Unit) -> Unit, done: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var recorder by remember { mutableStateOf<MediaRecorder?>(null) }
    var file by remember { mutableStateOf<File?>(null) }
    var started by remember { mutableLongStateOf(0L) }
    var seconds by remember { mutableLongStateOf(0L) }
    var problem by remember { mutableStateOf<String?>(null) }
    var filing by remember { mutableStateOf(false) }

    val start: () -> Unit = {
        val target = Voice.newFile(context)
        runCatching { recorder = Voice.record(context, target); file = target; started = System.currentTimeMillis() }
            .onFailure { problem = tr("Aufnahme geht nicht: {message}", "message" to it.message); target.delete() }
    }
    val askMic = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) start() else problem = tr("Ohne Mikrofon keine Aufnahme.")
    }
    // Ends the recording; the file if it holds anything.
    val stop: () -> File? = {
        runCatching { recorder?.stop() }
        recorder?.release()
        recorder = null
        file?.takeIf { it.length() > 0 } ?: run { file?.delete(); file = null; null }
    }
    val keep: (Boolean) -> Unit = { leaving ->
        val recorded = stop()
        if (recorded == null) {
            if (!leaving) done()
        } else {
            filing = true
            scope.launch {
                Voice.file(context, recorded, recorded.name, "audio/mp4")
                    .onSuccess {
                        Toast.makeText(context, tr("Sprachnotiz angelegt."), Toast.LENGTH_SHORT).show()
                        done()
                    }
                    // The recording stays in the cache; it can still be handed on.
                    .onFailure { filing = false; problem = tr("Sprachnotiz nicht gespeichert: {it}", "it" to said(it)) }
            }
        }
    }

    LaunchedEffect(Unit) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) start()
        else askMic.launch(Manifest.permission.RECORD_AUDIO)
    }
    LaunchedEffect(recorder) {
        while (recorder != null) {
            seconds = (System.currentTimeMillis() - started) / 1000
            delay(250)
        }
    }
    // Left while recording (home, screen off): nothing said is lost.
    val leave by rememberUpdatedState { if (recorder != null) keep(true) }
    DisposableEffect(Unit) {
        onLeft { leave() }
        onDispose { onLeft {}; recorder?.release() }
    }

    Column(
        Modifier.fillMaxSize().background(t.bg).safeDrawingPadding().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(tr("Sprachnotiz"), color = t.muted, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
        Text(
            if (recorder != null) "● %d:%02d".format(seconds / 60, seconds % 60) else if (filing) "…" else "■",
            color = if (recorder != null) t.overdue else t.fg, fontSize = 44.sp, fontWeight = FontWeight.SemiBold,
        )
        problem?.let { Text(it, color = t.overdue, fontSize = 14.sp) }
        if (recorder != null) {
            Button(onClick = { keep(false) }) { Text(tr("■ Stopp und anlegen"), fontSize = 18.sp) }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            TextButton(onClick = { stop()?.delete(); file?.delete(); done() }, enabled = !filing) { Text(tr("Aufnahme verwerfen")) }
            if (recorder != null || file != null) {
                OutlinedButton(
                    onClick = { (stop() ?: file)?.let { Voice.handOn(context, it) }; done() },
                    enabled = !filing,
                ) { Text(tr("Weitergeben …")) }
            }
        }
    }
}

/** Audio shared from another app becomes a voice note, as if recorded here (§8). */
@Composable
private fun SharedAudio(uri: Uri, done: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    var problem by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(uri) {
        val copied = withContext(Dispatchers.IO) {
            runCatching {
                val resolver = context.contentResolver
                val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                    if (c.moveToFirst()) c.getString(0) else null
                } ?: "voice.m4a"
                val type = resolver.getType(uri)?.takeIf { it.startsWith("audio/") } ?: "audio/mp4"
                val target = Voice.newFile(context)
                resolver.openInputStream(uri).use { input ->
                    requireNotNull(input) { tr("die Datei lässt sich nicht lesen") }
                    target.outputStream().use { input.copyTo(it) }
                }
                Triple(target, name.replace('/', '_'), type)
            }
        }
        copied
            .mapCatching { (file, name, type) ->
                Voice.file(context, file, name, type).onFailure { file.delete() }.getOrThrow()
            }
            .onSuccess {
                Toast.makeText(context, tr("Sprachnotiz angelegt."), Toast.LENGTH_SHORT).show()
                done()
            }
            .onFailure { problem = tr("Sprachnotiz nicht gespeichert: {it}", "it" to said(it)) }
    }
    Column(
        Modifier.fillMaxSize().background(t.bg).safeDrawingPadding().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(tr("Sprachnotiz"), color = t.muted, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
        Text(problem ?: "…", color = if (problem != null) t.overdue else t.fg, fontSize = 16.sp)
        if (problem != null) TextButton(onClick = done) { Text(tr("Schließen")) }
    }
}
