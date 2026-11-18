// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.ColorFilter
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.Image
import androidx.glance.ImageProvider
import androidx.glance.action.ActionParameters
import androidx.glance.action.actionParametersOf
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.action.ActionCallback
import androidx.glance.appwidget.action.actionRunCallback
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.lazy.LazyColumn
import androidx.glance.appwidget.lazy.items
import androidx.glance.appwidget.provideContent
import androidx.glance.appwidget.updateAll
import androidx.glance.background
import androidx.glance.color.ColorProvider as DayNight
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.padding
import androidx.glance.layout.size
import androidx.glance.layout.width
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import androidx.glance.unit.ColorProvider
import de.samudo.app.mobile.Due
import de.samudo.app.mobile.TaskRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The widget "Heute" (ADR 0028 §9; 08.11.2026): overdue, today and
 * the flagged available tasks, from `samu-mobile`'s `today`. The ring ticks
 * a task off through the outbox, "+" opens capture, a row opens the task.
 */
class TodayWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        val shown = withContext(Dispatchers.IO) { load(context) }
        provideContent { Today(context, shown) }
    }

    private fun load(context: Context): Shown = try {
        val samu = Store.get(context)
        if (!samu.status().signedIn) {
            Shown.SignedOut
        } else {
            val rows = samu.today()
            // These open in the app: the widget cannot ask (ADR 0128 §5).
            Shown.Rows(rows, rows.filter { it.group && samu.openBelow(it.id) > 0uL }.map { it.id }.toSet())
        }
    } catch (e: Exception) {
        Log.w("samu", "widget: $e")
        Shown.Failed
    }
}

private sealed interface Shown {
    data object SignedOut : Shown
    data object Failed : Shown
    /** [opening]: tasks with open tasks below, whose ring opens them. */
    data class Rows(val rows: List<TaskRow>, val opening: Set<String>) : Shown
}

/** The tokens of [Light] and [Dark], day and night as the system has it. */
internal object W {
    private fun of(pick: (Tokens) -> Color): ColorProvider = DayNight(day = pick(Light), night = pick(Dark))
    val bg = of { it.pane }
    val fg = of { it.fg }
    val muted = of { it.muted }
    val accent = of { it.accent }
    val overdue = of { it.overdue }
    val soon = of { it.soon }
    val flag = of { it.flag }
    val ok = of { it.ok }
    val okBg = of { it.okBg }
}

@Composable
private fun Today(context: Context, shown: Shown) {
    Column(GlanceModifier.fillMaxSize().background(W.bg).padding(12.dp)) {
        Row(GlanceModifier.fillMaxWidth().padding(start = 4.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                tr("Heute"),
                style = TextStyle(color = W.fg, fontSize = 17.sp, fontWeight = FontWeight.Bold),
                modifier = GlanceModifier.defaultWeight().clickable(actionStartActivity(open(context, null))),
            )
            Box(
                GlanceModifier.size(36.dp).clickable(actionStartActivity(open(context, CAPTURE))),
                contentAlignment = Alignment.Center,
            ) { Text("+", style = TextStyle(color = W.accent, fontSize = 24.sp, fontWeight = FontWeight.Bold)) }
        }
        when (shown) {
            Shown.SignedOut -> Note(context, tr("Anmelden in samudo"))
            Shown.Failed -> Note(context, tr("Nicht zu lesen – samudo öffnen"))
            is Shown.Rows -> if (shown.rows.isEmpty()) {
                Note(context, tr("Nichts für heute"))
            } else {
                LazyColumn {
                    items(shown.rows, itemId = { it.id.hashCode().toLong() }) { row -> Line(context, row, row.id in shown.opening) }
                }
            }
        }
    }
}

@Composable
private fun Note(context: Context, text: String) {
    Text(
        text,
        style = TextStyle(color = W.muted, fontSize = 14.sp),
        modifier = GlanceModifier.padding(4.dp).clickable(actionStartActivity(open(context, null))),
    )
}

@Composable
private fun Line(context: Context, row: TaskRow, opening: Boolean) {
    Row(GlanceModifier.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        // A habit here is ticked for today, never completed (ADR 0092 §2).
        val habit = row.habit
        if (habit != null) {
            HabitButton(row.id, habit)
        } else Box(
            GlanceModifier.size(32.dp).clickable(
                if (opening) {
                    actionStartActivity(open(context, TASK, row.id))
                } else {
                    actionRunCallback<CompleteAction>(actionParametersOf(CompleteAction.ID to row.id))
                },
            ),
            contentAlignment = Alignment.Center,
        ) {
            Image(
                ImageProvider(R.drawable.widget_ring),
                contentDescription = tr("Erledigen"),
                modifier = GlanceModifier.size(20.dp),
                colorFilter = ColorFilter.tint(if (row.dueState == Due.OVERDUE) W.overdue else W.muted),
            )
        }
        Spacer(GlanceModifier.width(8.dp))
        Column(GlanceModifier.defaultWeight().clickable(actionStartActivity(open(context, TASK, row.id)))) {
            Text(row.title, style = TextStyle(color = W.fg, fontSize = 14.sp), maxLines = 1)
            val detail = Words.detail(row)
            if (detail.isNotEmpty()) {
                val color = when (row.dueState) {
                    Due.OVERDUE -> W.overdue
                    Due.SOON -> W.soon
                    else -> W.muted
                }
                Text(detail, style = TextStyle(color = color, fontSize = 12.sp), maxLines = 1)
            }
        }
        if (row.flagged) Text("⚑", style = TextStyle(color = W.flag, fontSize = 14.sp))
    }
}

/** What a tap in the widget asks of [MainActivity], as the intent's data. */
const val WIDGET_SCHEME = "samu-widget"
const val CAPTURE = "capture"
const val TASK = "task"

/** An occurrence of an appointment, by the key of its notice (ADR 0101 §7). */
const val APPOINTMENT = "appointment"
/** The launcher's dot opens the inbox: `samu-widget:inbox`. */
const val INBOX = "inbox"

/** A data URI of its own per row, so the system keeps the intents apart. */
internal fun open(context: Context, what: String?, id: String? = null): Intent =
    Intent(context, MainActivity::class.java)
        .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        .apply { if (what != null) data = Uri.parse("$WIDGET_SCHEME:$what" + (id?.let { "/$it" } ?: "")) }

/** The ring: done in the outbox, then a sync and the widget drawn again. */
class CompleteAction : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        val id = parameters[ID] ?: return
        withContext(Dispatchers.IO) {
            try {
                // Open tasks below since it was drawn: nothing here, the
                // ring opens the task once drawn again (ADR 0128 §5).
                val samu = Store.get(context)
                if (samu.openBelow(id) == 0uL) samu.closeTasks(listOf(id), "completed", "close")
            } catch (e: Exception) {
                Log.w("samu", "widget complete: $e")
            }
        }
        Sync.afterChange(context)
        Reminders.arm(context)
        Clocking.refresh(context)
        Widgets.refresh(context)
    }

    companion object {
        val ID = ActionParameters.Key<String>("task")
    }
}

class TodayWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = TodayWidget()
}

object Widgets {
    /**
     * Draw every "Heute" and "Habits" widget anew: after a sync, on leaving
     * the app and at midnight ([Midnight]).
     * The launcher hears of it at the same moments ([Launcher]).
     */
    suspend fun refresh(context: Context) {
        try {
            TodayWidget().updateAll(context)
            HabitsWidget().updateAll(context)
        } catch (e: Exception) {
            Log.w("samu", "widget refresh: $e")
        }
        Midnight.arm(context)
        Launcher.tell(context)
    }
}
