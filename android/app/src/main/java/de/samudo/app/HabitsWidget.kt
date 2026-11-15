package de.samudo.app

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.action.ActionParameters
import androidx.glance.action.actionParametersOf
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.action.ActionCallback
import androidx.glance.appwidget.action.actionRunCallback
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.lazy.LazyColumn
import androidx.glance.appwidget.lazy.items
import androidx.glance.appwidget.provideContent
import androidx.glance.background
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.layout.width
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import de.samudo.app.mobile.HabitLine
import de.samudo.app.mobile.HabitRow
import java.time.LocalDate
import java.time.ZoneId
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The widget "Habits" (ADR 0092 §4): every open habit, without a scope, in
 * the order of project and position. ✓ or +1 enters today done through the
 * outbox; a habit to avoid shows "frei seit n T" without a button, so that
 * no relapse comes from the home screen by mistake. The title opens the
 * task with its section "Serie", where a wrong tap is taken back.
 */
class HabitsWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        val shown = withContext(Dispatchers.IO) { load(context) }
        provideContent { Habits(context, shown) }
    }

    private fun load(context: Context): HabitsShown = try {
        val samu = Store.get(context)
        if (!samu.status().signedIn) HabitsShown.SignedOut else HabitsShown.Lines(samu.habits())
    } catch (e: Exception) {
        Log.w("samu", "habits widget: $e")
        HabitsShown.Failed
    }
}

private sealed interface HabitsShown {
    data object SignedOut : HabitsShown
    data object Failed : HabitsShown
    data class Lines(val lines: List<HabitLine>) : HabitsShown
}

@Composable
private fun Habits(context: Context, shown: HabitsShown) {
    Column(GlanceModifier.fillMaxSize().background(W.bg).padding(12.dp)) {
        Text(
            "Habits",
            style = TextStyle(color = W.fg, fontSize = 17.sp, fontWeight = FontWeight.Bold),
            modifier = GlanceModifier.fillMaxWidth().padding(start = 4.dp, bottom = 6.dp)
                .clickable(actionStartActivity(open(context, null))),
        )
        when (shown) {
            HabitsShown.SignedOut -> Hint(context, tr("Anmelden in samudo"))
            HabitsShown.Failed -> Hint(context, tr("Nicht zu lesen – samudo öffnen"))
            is HabitsShown.Lines -> if (shown.lines.isEmpty()) {
                Hint(context, tr("Keine Habits"))
            } else {
                LazyColumn {
                    items(shown.lines, itemId = { it.id.hashCode().toLong() }) { line -> HabitWidgetLine(context, line) }
                }
            }
        }
    }
}

@Composable
private fun Hint(context: Context, text: String) {
    Text(
        text,
        style = TextStyle(color = W.muted, fontSize = 14.sp),
        modifier = GlanceModifier.padding(4.dp).clickable(actionStartActivity(open(context, null))),
    )
}

@Composable
private fun HabitWidgetLine(context: Context, line: HabitLine) {
    val h = line.habit
    // Met: the row steps back, the ✓ turns green.
    val title = if (h.met) W.muted else W.fg
    Row(GlanceModifier.fillMaxWidth().padding(vertical = 5.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(GlanceModifier.defaultWeight().clickable(actionStartActivity(open(context, TASK, line.id)))) {
            Text(line.title, style = TextStyle(color = title, fontSize = 14.sp), maxLines = 1)
            val figures = listOfNotNull(Words.habitCount(h), Words.habitStreak(h)).joinToString(" · ")
            if (figures.isNotEmpty()) {
                val color = when {
                    h.kind == "avoid" && h.today == "fail" -> W.overdue
                    h.kind == "avoid" || h.met -> W.ok
                    else -> W.muted
                }
                Text(figures, style = TextStyle(color = color, fontSize = 12.sp), maxLines = 1)
            }
        }
        if (h.kind != "avoid") {
            Spacer(GlanceModifier.width(8.dp))
            HabitButton(line.id, h)
        }
    }
}

/** ✓ or +1, as in the app's row; for the widgets "Habits" and "Heute". */
@Composable
internal fun HabitButton(id: String, h: HabitRow) {
    if (h.kind == "avoid") {
        Spacer(GlanceModifier.width(32.dp))
        return
    }
    val counts = Words.habitCounts(h) && !h.met
    Box(
        GlanceModifier.width(44.dp).height(32.dp).cornerRadius(16.dp)
            .background(if (h.met) W.okBg else W.bg)
            .clickable(actionRunCallback<HabitAction>(actionParametersOf(HabitAction.ID to id))),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            if (counts) "+1" else "✓",
            style = TextStyle(color = if (h.met) W.ok else W.accent, fontSize = 16.sp, fontWeight = FontWeight.Bold),
        )
    }
}

/** ✓ or +1: an entry `done` for today in the outbox, then sync and draw. */
class HabitAction : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        val id = parameters[ID] ?: return
        withContext(Dispatchers.IO) {
            try {
                Store.get(context).enterHabit(id, "done", null, null)
            } catch (e: Exception) {
                Log.w("samu", "widget habit: $e")
            }
        }
        Sync.afterChange(context)
        Widgets.refresh(context)
    }

    companion object {
        val ID = ActionParameters.Key<String>("habit")
    }
}

class HabitsWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = HabitsWidget()
}

/**
 * Midnight in the account's zone (ADR 0092 §4): a new day begins and the
 * ticks go back, so the widgets are drawn anew. Set like the reminders'
 * alarm, but inexact: a minute late does no harm.
 */
object Midnight {
    private const val REQUEST = 7301

    fun arm(context: Context) {
        val context = context.applicationContext
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val zone = runCatching { ZoneId.of(Store.get(context).status().zone) }.getOrDefault(ZoneId.systemDefault())
                val next = LocalDate.now(zone).plusDays(1).atStartOfDay(zone).plusMinutes(1).toInstant().toEpochMilli()
                val manager = context.getSystemService(AlarmManager::class.java)
                manager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next, intent(context))
            } catch (e: Exception) {
                Log.w("samu", "midnight arm: $e")
            }
        }
    }

    private fun intent(context: Context): PendingIntent =
        PendingIntent.getBroadcast(
            context,
            REQUEST,
            Intent(context, MidnightReceiver::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
}

/** Draws the widgets anew at midnight; [Widgets.refresh] sets the next one. */
class MidnightReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val done = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                Widgets.refresh(context)
            } finally {
                done.finish()
            }
        }
    }
}
