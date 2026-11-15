package de.samudo.app

import android.Manifest
import android.content.ActivityNotFoundException
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.CalendarContract
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import de.samudo.app.mobile.AppointmentRow
import de.samudo.app.mobile.Block
import de.samudo.app.mobile.CalendarDay
import de.samudo.app.mobile.CalendarPage
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/**
 * Hands appointments to the phone's calendar app (ADR 0049): samu reads
 * calendars only; editing, deleting and making new ones happens there.
 */
object CalendarApp {
    private const val DAY_MS = 86_400_000L

    fun canRead(context: Context): Boolean =
        ContextCompat.checkSelfPermission(context, Manifest.permission.READ_CALENDAR) == PackageManager.PERMISSION_GRANTED

    /**
     * The appointment in the calendar app: found by its UID among the
     * phone's instances around its time, else the calendar at its day
     * (ADR 0049 §2).
     */
    fun open(context: Context, a: AppointmentRow) {
        val found = if (canRead(context)) runCatching { find(context, a) }.getOrNull() else null
        val intent = if (found != null) {
            Intent(Intent.ACTION_VIEW, ContentUris.withAppendedId(CalendarContract.Events.CONTENT_URI, found.first))
                .putExtra(CalendarContract.EXTRA_EVENT_BEGIN_TIME, found.second)
                .putExtra(CalendarContract.EXTRA_EVENT_END_TIME, found.third)
        } else {
            atDay(a)
        }
        start(context, intent)
        if (found == null && canRead(context)) {
            Toast.makeText(context, tr("Termin nicht im Kalender des Handys; der Tag ist offen."), Toast.LENGTH_SHORT).show()
        }
    }

    /** A new appointment in the calendar app, which asks for the calendar (ADR 0049 §4). */
    fun insert(context: Context, beginMs: Long, endMs: Long, allDay: Boolean) {
        start(
            context,
            Intent(Intent.ACTION_INSERT, CalendarContract.Events.CONTENT_URI)
                .putExtra(CalendarContract.EXTRA_EVENT_BEGIN_TIME, beginMs)
                .putExtra(CalendarContract.EXTRA_EVENT_END_TIME, endMs)
                .putExtra(CalendarContract.EXTRA_EVENT_ALL_DAY, allDay),
        )
    }

    /** A new appointment over the whole day `date` (`2026-11-20`), from Forecast. */
    fun insertDay(context: Context, date: String) {
        val begin = LocalDate.parse(date).atStartOfDay(ZoneId.systemDefault()).toInstant().toEpochMilli()
        insert(context, begin, begin + DAY_MS, allDay = true)
    }

    /** Event id, begin and end of the instance with the UID nearest to its time. */
    private fun find(context: Context, a: AppointmentRow): Triple<Long, Long, Long>? {
        if (a.uid.isEmpty()) return null
        val uri = CalendarContract.Instances.CONTENT_URI.buildUpon()
            .let { ContentUris.appendId(it, a.beginMs - DAY_MS) }
            .let { ContentUris.appendId(it, a.endMs + DAY_MS) }
            .build()
        val columns = arrayOf(
            CalendarContract.Instances.EVENT_ID,
            CalendarContract.Instances.BEGIN,
            CalendarContract.Instances.END,
        )
        context.contentResolver.query(
            uri, columns, "${CalendarContract.Events.UID_2445} = ?", arrayOf(a.uid), null,
        )?.use { c ->
            var best: Triple<Long, Long, Long>? = null
            while (c.moveToNext()) {
                val hit = Triple(c.getLong(0), c.getLong(1), c.getLong(2))
                if (best == null || kotlin.math.abs(hit.second - a.beginMs) < kotlin.math.abs(best.second - a.beginMs)) {
                    best = hit
                }
            }
            return best
        }
        return null
    }

    /** The calendar app at the appointment's day. */
    private fun atDay(a: AppointmentRow): Intent {
        val at = if (a.allDay) {
            LocalDate.parse(a.date).atTime(12, 0).atZone(ZoneId.systemDefault()).toInstant().toEpochMilli()
        } else {
            a.beginMs
        }
        val uri = CalendarContract.CONTENT_URI.buildUpon().appendPath("time")
            .let { ContentUris.appendId(it, at) }.build()
        return Intent(Intent.ACTION_VIEW, uri)
    }

    private fun start(context: Context, intent: Intent) {
        try {
            context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        } catch (_: ActivityNotFoundException) {
            Toast.makeText(context, tr("Keine Kalender-App gefunden."), Toast.LENGTH_SHORT).show()
        }
    }

    /** When an appointment is, in words: `Do 20.11. 09:00–09:45`, `Do 20.11. – Sa 22.11., ganztägig`. */
    fun whenWords(a: AppointmentRow): String {
        val start = LocalDate.parse(a.date)
        if (a.allDay) {
            val days = ((a.endMs - a.beginMs) / DAY_MS).coerceAtLeast(1)
            val last = start.plusDays(days - 1)
            return if (days == 1L) tr("{n}, ganztägig", "n" to dayWords(start)) else tr("{n} – {n2}, ganztägig", "n" to dayWords(start), "n2" to dayWords(last))
        }
        val times = listOfNotNull(a.start, a.end?.takeIf { it != a.start }).joinToString("–")
        return "${dayWords(start)} $times"
    }

    fun dayWords(d: LocalDate): String =
        DateTimeFormatter.ofPattern(if (I18n.lang == "de") "EE d.M." else "EEE d MMM", I18n.locale()).format(d).replace(".,", "")
}

/** The calendar's colour as a Compose colour, or the muted one. */
@Composable
private fun calendarColor(hex: String?): Color {
    val t = LocalTokens.current
    return hex?.let { runCatching { Color(android.graphics.Color.parseColor(it)) }.getOrNull() } ?: t.muted
}

/**
 * An appointment opened (ADR 0049 §2): what samu knows of it, then „In
 * Kalender-App öffnen“, which asks for READ_CALENDAR the first time and
 * without it opens the day, and the link to the original if there is one.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppointmentSheet(a: AppointmentRow, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val t = LocalTokens.current
    val uri = LocalUriHandler.current
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        CalendarApp.open(context, a)
        onDismiss()
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = t.pane) {
        Column(Modifier.fillMaxWidth().padding(20.dp, 0.dp, 20.dp, 28.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(tr("Termin · nur lesbar"), color = t.muted, fontSize = 12.sp)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Box(Modifier.size(12.dp).background(calendarColor(a.color), CircleShape))
                Text(a.title + if (a.tentative) tr(" (vorläufig)") else "", color = t.fg, fontSize = 20.sp, fontWeight = FontWeight.SemiBold)
            }
            Text(CalendarApp.whenWords(a), color = t.fg)
            Text(a.calendar, color = t.muted)
            a.location?.let { Text(it, color = t.fg) }
            // What the phone will do (ADR 0101 §9), also with the switch off.
            if (a.alarms.isNotEmpty()) {
                Text(tr("Erinnerung: ") + a.alarms.joinToString(", ") { Words.alarm(it, a) }, color = t.muted, fontSize = 14.sp)
            }
            a.note?.let { Text(it, color = t.muted, fontSize = 14.sp) }
            Spacer(Modifier.height(4.dp))
            Button(
                onClick = {
                    if (CalendarApp.canRead(context)) {
                        CalendarApp.open(context, a)
                        onDismiss()
                    } else {
                        ask.launch(Manifest.permission.READ_CALENDAR)
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            ) { Text(tr("In Kalender-App öffnen")) }
            a.link?.let { link ->
                OutlinedButton(onClick = { runCatching { uri.openUri(link) }; onDismiss() }, modifier = Modifier.fillMaxWidth()) {
                    Text(tr("Im Original öffnen ↗"))
                }
            }
        }
    }
}

/** How Forecast shows: the list, or the calendar as a day or a week (ADR 0047 §11). */
enum class ForecastMode {
    LIST, DAY, WEEK;

    /** In the language of the moment, so not fixed when the enum is made. */
    val title: String
        get() = when (this) {
            LIST -> tr("Liste")
            DAY -> tr("Ansicht|Tag")
            WEEK -> tr("Ansicht|Woche")
        }
}

@Composable
fun ForecastModes(mode: ForecastMode, choose: (ForecastMode) -> Unit) {
    val t = LocalTokens.current
    Row(Modifier.padding(16.dp, 0.dp, 16.dp, 4.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        ForecastMode.entries.forEach { m ->
            Text(
                m.title,
                color = if (m == mode) t.onAccent else t.fg,
                fontSize = 13.sp,
                modifier = Modifier
                    .background(if (m == mode) t.accent else t.side, RoundedCornerShape(14.dp))
                    .clickable { choose(m) }
                    .padding(12.dp, 5.dp),
            )
        }
    }
}

private val HOUR = 48.dp
private const val SLOT = 30

/**
 * The calendar view in the app (ADR 0047 §11, ADR 0049): a day or the week
 * from Monday, laid out by samu-mobile. A tap on an appointment opens its
 * sheet, on a task the task; a long press on free time starts a new
 * appointment there in the calendar app, an hour long.
 */
@Composable
fun CalendarView(tick: Int, week: Boolean, onDay: () -> Unit, open: (String) -> Unit) {
    val t = LocalTokens.current
    val context = LocalContext.current
    var day by rememberSaveable { mutableStateOf<String?>(null) }
    var shown by remember { mutableStateOf<AppointmentRow?>(null) }
    val (page, error) = loaded<CalendarPage>(tick, day to week) { it.calendar(day, week) }
    error?.let { Text(it, color = t.overdue, modifier = Modifier.padding(16.dp)) }
    val p = page ?: return
    val scroll = rememberScrollState()
    val density = LocalDensity.current
    LaunchedEffect(Unit) { scroll.scrollTo(with(density) { (HOUR * 7).roundToPx() }) }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(8.dp, 0.dp), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = { day = p.previous }) { Text("‹", fontSize = 20.sp) }
            TextButton(onClick = { day = null }) { Text(tr("Heute")) }
            TextButton(onClick = { day = p.next }) { Text("›", fontSize = 20.sp) }
            Text(heading(p), color = t.fg, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 8.dp))
        }
        // Day heads and the band of whole days.
        Row(Modifier.fillMaxWidth().padding(end = 4.dp)) {
            Spacer(Modifier.width(GUTTER))
            p.days.forEach { d ->
                Column(Modifier.weight(1f).padding(horizontal = 1.dp)) {
                    if (week) {
                        Text(
                            dayHead(d),
                            color = if (d.today) t.accent else t.muted,
                            fontSize = 11.sp,
                            fontWeight = if (d.today) FontWeight.Bold else FontWeight.Normal,
                            maxLines = 1,
                            modifier = Modifier.fillMaxWidth().clickable {
                                day = d.date
                                onDay()
                            }.padding(vertical = 4.dp),
                        )
                    }
                    d.allDay.forEach { a ->
                        Text(
                            a.title,
                            color = t.fg, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.fillMaxWidth().padding(vertical = 1.dp)
                                .background(calendarColor(a.color).copy(alpha = 0.25f), RoundedCornerShape(4.dp))
                                .clickable { shown = a }
                                .padding(4.dp, 2.dp),
                        )
                    }
                }
            }
        }
        Box(Modifier.fillMaxWidth().height(1.dp).background(t.line))
        Row(Modifier.fillMaxSize().verticalScroll(scroll).padding(end = 4.dp)) {
            Column(Modifier.width(GUTTER)) {
                (0 until 24).forEach { h ->
                    Text(
                        if (h == 0) "" else "%02d:00".format(h),
                        color = t.muted, fontSize = 10.sp,
                        modifier = Modifier.height(HOUR).padding(end = 4.dp).offset(y = (-6).dp),
                    )
                }
            }
            p.days.forEach { d ->
                DayColumn(d, Modifier.weight(1f), { shown = it }, open) { minute ->
                    val begin = d.startMs + minute * 60_000L
                    CalendarApp.insert(context, begin, begin + 3_600_000L, allDay = false)
                }
            }
        }
    }
    shown?.let { AppointmentSheet(it) { shown = null } }
}

private val GUTTER = 40.dp

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun DayColumn(
    d: CalendarDay,
    modifier: Modifier,
    show: (AppointmentRow) -> Unit,
    open: (String) -> Unit,
    newAt: (Int) -> Unit,
) {
    val t = LocalTokens.current
    val density = LocalDensity.current
    BoxWithConstraints(
        modifier
            .height(HOUR * 24)
            .border(0.5.dp, t.rowline)
            .background(if (d.today) t.select.copy(alpha = 0.35f) else Color.Transparent)
            .pointerInput(d.date) {
                detectTapGestures(onLongPress = { at ->
                    val hour = with(density) { HOUR.toPx() }
                    val minute = (at.y / hour * 60).toInt()
                    newAt((minute / SLOT * SLOT).coerceIn(0, 24 * 60 - SLOT))
                })
            },
    ) {
        val width = maxWidth
        // The hour lines.
        (1 until 24).forEach { h ->
            Box(Modifier.offset(y = HOUR * h).fillMaxWidth().height(0.5.dp).background(t.rowline))
        }
        d.blocks.forEach { b -> BlockBox(b, width, show, open) }
        d.now?.let { minute ->
            Box(Modifier.offset(y = HOUR * (minute.toFloat() / 60f)).fillMaxWidth().height(2.dp).background(t.overdue))
        }
    }
}

@Composable
private fun BlockBox(b: Block, width: androidx.compose.ui.unit.Dp, show: (AppointmentRow) -> Unit, open: (String) -> Unit) {
    val t = LocalTokens.current
    val lanes = b.lanes.toInt().coerceAtLeast(1)
    val lane = b.lane.toInt()
    val top = HOUR * (b.top.toFloat() / 60f)
    val height = (HOUR * ((b.bottom - b.top).toFloat() / 60f) - 2.dp).coerceAtLeast(12.dp)
    val laneWidth = width / lanes
    val a = b.appointment
    val task = b.task
    val color = if (a != null) calendarColor(a.color) else t.accent
    val done = task != null && (task.done || task.dropped)
    Column(
        Modifier
            .offset(x = laneWidth * lane + 1.dp, y = top)
            .width(laneWidth - 2.dp)
            .height(height)
            .background(color.copy(alpha = if (a != null) 0.3f else 0.12f), RoundedCornerShape(4.dp))
            .border(1.dp, color.copy(alpha = if (a?.tentative == true || task != null) 0.6f else 0f), RoundedCornerShape(4.dp))
            .clickable {
                if (a != null) show(a) else if (task != null) open(task.id)
            }
            .padding(3.dp, 1.dp),
    ) {
        Text(
            b.label + if (task != null) " ☐" else "",
            color = t.muted, fontSize = 10.sp, maxLines = 1,
        )
        Text(
            a?.title ?: task?.title ?: "",
            color = t.fg, fontSize = 11.sp, lineHeight = 13.sp,
            maxLines = if (b.bottom - b.top < 50u) 1 else 3, overflow = TextOverflow.Ellipsis,
            textDecoration = if (done) TextDecoration.LineThrough else null,
        )
    }
}

private fun longDay() = DateTimeFormatter.ofPattern(if (I18n.lang == "de") "EEEE, d. MMMM" else "EEEE d MMMM", I18n.locale())
private fun monthYear() = DateTimeFormatter.ofPattern("MMMM yyyy", I18n.locale())

private fun heading(p: CalendarPage): String {
    val longDay = longDay()
    val monthYear = monthYear()
    val first = LocalDate.parse(p.days.first().date)
    if (p.days.size == 1) return longDay.format(first)
    val a = monthYear.format(first)
    val b = monthYear.format(LocalDate.parse(p.days.last().date))
    return if (a == b) a else "$a – $b"
}

private fun dayHead(d: CalendarDay): String =
    DateTimeFormatter.ofPattern(if (I18n.lang == "de") "EE d." else "EEE d", I18n.locale())
        .format(LocalDate.parse(d.date)).replace(".,", "")
