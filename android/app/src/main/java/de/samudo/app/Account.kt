// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.content.Context
import android.util.Log
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** One account on this phone, as the list shows it (ADR 0107 §4). */
data class AccountLine(
    val slot: String,
    val account: String?,
    val server: String?,
    val signedIn: Boolean,
    val outbox: UInt,
    val active: Boolean,
) {
    /** `konto · samu.example.org`, or the folder while it holds no account yet. */
    val label: String
        get() = listOfNotNull(account, server?.substringAfter("://")).joinToString(" · ").ifEmpty { slot }
}

/**
 * The accounts on the phone (ADR 0107): switching, adding, signing out.
 * Only the active one is open; the others are read from their folders.
 */
object Accounts {
    /** The active folder as Compose state: the main screen starts anew with it. */
    val current = mutableStateOf<String?>(null)

    /** While an account is being added: the folder to go back to on „Abbrechen“. */
    val addingFrom = mutableStateOf<String?>(null)

    fun list(context: Context): List<AccountLine> {
        val active = Store.active(context)
        return Store.slots(context).mapNotNull { slot ->
            val status = Store.status(context, slot) ?: return@mapNotNull null
            AccountLine(slot, status.account, status.server, status.signedIn, status.outbox, slot == active)
        }
    }

    /**
     * Changes to another account (§3): reminders, the clock, the widgets
     * and the sync follow the new one; what the one left has in its outbox
     * gets one more try to go out.
     */
    fun switch(context: Context, slot: String, done: () -> Unit = {}) {
        val context = context.applicationContext
        CoroutineScope(Dispatchers.IO).launch {
            val left = Store.switch(context, slot)
            withContext(Dispatchers.Main) {
                current.value = slot
                done()
                redrawApp()
            }
            followActive(context)
            if (left != null) {
                try {
                    if (left.status().signedIn && left.outboxCount() > 0u) left.sync()
                } catch (e: Exception) {
                    Log.i("samu", "last sync of the account left: $e")
                }
            }
        }
    }

    /** A new, empty folder becomes active and shows the sign-in (§4). */
    fun add(context: Context) {
        addingFrom.value = Store.active(context)
        switch(context, Store.fresh(context))
    }

    /**
     * „Abbrechen“ while adding: back, and the folder goes. After a restart
     * the one to go back to is forgotten; then the first account there is.
     */
    fun cancelAdding(context: Context) {
        val fresh = Store.active(context)
        val from = addingFrom.value
            ?: list(context).firstOrNull { !it.active && it.account != null }?.slot
            ?: return
        addingFrom.value = null
        switch(context, from) {
            CoroutineScope(Dispatchers.IO).launch { runCatching { Store.remove(context, fresh) } }
        }
    }

    /** Signed in: adding is over. */
    fun signedIn(context: Context) {
        addingFrom.value = null
        followActive(context.applicationContext)
    }

    /** Signs the active account out (§4); its mirror and outbox stay. */
    suspend fun logout(context: Context) {
        withContext(Dispatchers.IO) { Store.get(context).logout() }
        followActive(context.applicationContext)
        redrawApp()
    }

    private fun followActive(context: Context) {
        Reminders.arm(context)
        Clocking.refresh(context)
        CoroutineScope(Dispatchers.IO).launch { Widgets.refresh(context) }
        Sync.now(context)
    }
}

/**
 * Settings › Konto (ADR 0107 §4): the active account with „Abmelden“, the
 * accounts on this phone to switch to or remove, and „Konto hinzufügen“.
 */
@Composable
fun AccountSection(tick: Int, back: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var lines by remember { mutableStateOf<List<AccountLine>?>(null) }
    var status by remember { mutableStateOf<de.samudo.app.mobile.Status?>(null) }
    var reload by remember { mutableStateOf(0) }
    // A question before signing out or removing, with what it would cost.
    var asking by remember { mutableStateOf<Pair<String, () -> Unit>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(tick, reload) {
        withContext(Dispatchers.IO) {
            status = runCatching { Store.get(context).status() }.getOrNull()
            lines = Accounts.list(context)
        }
    }
    val signOut: () -> Unit = {
        scope.launch {
            try {
                Accounts.logout(context)
            } catch (e: Exception) {
                error = said(e)
            }
        }
    }
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item { TextButton(onClick = back) { Text(tr("‹ Zurück")) } }
        status?.let { s ->
            item {
                Column(
                    Modifier.fillMaxWidth().background(t.side, RoundedCornerShape(8.dp)).padding(14.dp, 12.dp),
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    Text(s.account.orEmpty(), color = t.fg, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
                    Text(s.server.orEmpty(), color = t.muted, fontSize = 13.sp)
                    Text(tr("Gerät: {device}", "device" to s.device.orEmpty()), color = t.muted, fontSize = 13.sp)
                    s.lastSync?.let { Text(tr("Letzter Sync: {at}", "at" to it.take(16).replace('T', ' ')), color = t.muted, fontSize = 13.sp) }
                    if (s.outbox > 0u) Text(tr("{outbox} im Ausgang", "outbox" to s.outbox), color = t.wait, fontSize = 13.sp)
                }
            }
            item {
                OutlinedButton(
                    onClick = {
                        if (s.outbox > 0u) {
                            asking = tr(
                                "{outbox} Änderungen sind noch nicht gesendet. Sie bleiben und gehen beim nächsten Anmelden an dieses Konto hinaus.",
                                "outbox" to s.outbox,
                            ) to signOut
                        } else {
                            signOut()
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(tr("Abmelden")) }
            }
        }
        error?.let { item { Text(it, color = t.overdue, fontSize = 13.sp) } }
        item { Heading(tr("Konten auf diesem Handy")) }
        items(lines.orEmpty(), key = { it.slot }) { line ->
            Row(
                Modifier.fillMaxWidth()
                    .background(t.side, RoundedCornerShape(8.dp))
                    .clickable(enabled = !line.active) { Accounts.switch(context, line.slot) }
                    .padding(14.dp, 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text((if (line.active) "✓ " else "") + line.label, color = t.fg, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                    val hints = buildList {
                        if (!line.signedIn) add(tr("abgemeldet"))
                        if (line.outbox > 0u) add(tr("{outbox} im Ausgang", "outbox" to line.outbox))
                    }
                    if (hints.isNotEmpty()) Text(hints.joinToString(" · "), color = t.muted, fontSize = 12.sp)
                }
                if (!line.active) {
                    TextButton(onClick = {
                        val remove = {
                            scope.launch {
                                withContext(Dispatchers.IO) { runCatching { Store.remove(context, line.slot) } }
                                reload++
                                // The main screen counts the accounts too: the line
                                // under its title goes with the last other one.
                                redrawApp()
                            }
                            Unit
                        }
                        asking = (
                            if (line.outbox > 0u) {
                                tr("{label} entfernen? {outbox} Änderungen sind noch nicht gesendet und gehen verloren.", "label" to line.label, "outbox" to line.outbox)
                            } else {
                                tr("{label} von diesem Handy entfernen?", "label" to line.label)
                            }
                            ) to remove
                    }) { Text(tr("Entfernen"), color = t.overdue) }
                }
            }
        }
        item {
            OutlinedButton(onClick = { Accounts.add(context) }, modifier = Modifier.fillMaxWidth()) {
                Text(tr("+ Konto hinzufügen"))
            }
        }
    }
    asking?.let { (text, go) ->
        AlertDialog(
            onDismissRequest = { asking = null },
            text = { Text(text) },
            confirmButton = { TextButton(onClick = { asking = null; go() }) { Text("OK") } },
            dismissButton = { TextButton(onClick = { asking = null }) { Text(tr("Abbrechen")) } },
        )
    }
}
