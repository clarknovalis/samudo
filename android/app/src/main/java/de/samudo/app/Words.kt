// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import de.samudo.app.mobile.AlarmRow
import de.samudo.app.mobile.Anchor
import de.samudo.app.mobile.AppointmentRow
import de.samudo.app.mobile.Chip
import de.samudo.app.mobile.ClockRow
import de.samudo.app.mobile.Day
import de.samudo.app.mobile.Due
import de.samudo.app.mobile.HabitEntryRow
import de.samudo.app.mobile.HabitRow
import de.samudo.app.mobile.Notice
import de.samudo.app.mobile.ReasonRow
import de.samudo.app.mobile.ReminderRow
import de.samudo.app.mobile.TaskRow
import de.samudo.app.mobile.WarningRow
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.TextStyle

/**
 * What samu-mobile computed, in words (ADR 0020 §2, §5). Nothing here
 * decides anything; the codes come from samu-core, as in the web UI's
 * words.js.
 */
object Words {
    /** The parts of the task page a scope can show (ADR 0072 §2), in order. */
    val PARTS get() = listOf(
        "properties" to "Properties", "availability" to tr("Verfügbarkeit"), "state" to tr("Zustand"),
        "project" to tr("Projekt"), "priority" to tr("Priorität"), "defer" to tr("Start"), "due" to tr("Fällig"),
        "planned" to tr("Geplant"), "repeat" to tr("Wiederholung"), "series" to tr("Serie"), "estimate" to tr("Schätzung"), "tags" to "Tags",
        "note" to tr("Notiz"), "checklist" to tr("Checkliste"), "links" to "Links", "reminders" to tr("Erinnerungen"), "clock" to "Clocking",
        "comments" to tr("Kommentare"), "attachments" to tr("Anhänge"), "logbook" to "Logbook",
    )

    /** A habit counts taps rather than ticking once (ADR 0092 §2): +1. */
    fun habitCounts(h: HabitRow): Boolean = h.unit != null || h.target > 1u

    /** Beside the mark: `5/8 Glas`, or for `avoid` "frei seit n T". */
    fun habitCount(h: HabitRow): String? = when {
        h.kind == "avoid" -> tr("frei seit {streak} T", "streak" to h.streak)
        habitCounts(h) -> "${h.amount}/${h.target}" + (h.unit?.let { " $it" } ?: "")
        else -> null
    }

    /** The streak as a small figure; none at nought and for `avoid`. */
    fun habitStreak(h: HabitRow): String? =
        if (h.kind == "avoid" || h.streak == 0u) null else "🔥${h.streak}"

    /** The goal: "täglich", "3× je Woche", "8 Glas am Tag". */
    fun habitGoal(h: HabitRow): String {
        if (h.kind == "avoid") return tr("vermeiden")
        val per = when (h.per) { "week" -> tr("je Woche"); "month" -> tr("je Monat"); else -> tr("am Tag") }
        return when {
            h.unit != null -> "${h.target} ${h.unit} $per"
            h.target == 1u && h.per == "day" -> tr("täglich")
            else -> "${h.target}× $per"
        }
    }

    /** What a day of a habit is (ADR 0091 §3). */
    fun habitDay(state: String, avoid: Boolean): String = when (state) {
        "done" -> if (avoid) tr("frei") else tr("geschafft")
        "partial" -> tr("teilweise")
        "fail" -> if (avoid) tr("Rückfall") else tr("nicht geschafft")
        "pause" -> tr("Pause")
        else -> tr("nicht erfasst")
    }

    /** One entry: "geschafft +2", "Rückfall", "Pause bis Fr 20.11.". */
    fun habitEntry(e: HabitEntryRow, avoid: Boolean): String = when (e.status) {
        "done" -> if (avoid) tr("frei ab hier") else tr("geschafft") + (e.amount?.let { " +$it" } ?: "")
        "fail" -> if (avoid) tr("Rückfall") else tr("nicht geschafft")
        "pause" -> tr("Pause") + (e.until?.let { tr(" bis {n}", "n" to day(it)) } ?: "")
        else -> e.status
    } + if (e.pending) tr(" · nicht gesynct") else ""

    fun part(key: String): String = PARTS.firstOrNull { it.first == key }?.second ?: key

    /**
     * "Mi 04.11." from "2026-11-04", in English "Wed 4 Nov", with " 09:00"
     * when there is a time.
     */
    fun day(text: String?): String {
        if (text.isNullOrBlank()) return ""
        val date = runCatching { LocalDate.parse(text.take(10)) }.getOrNull() ?: return text
        val weekday = date.dayOfWeek.getDisplayName(TextStyle.SHORT, I18n.locale()).trimEnd('.')
        val day = if (I18n.lang == "de") {
            "%s %02d.%02d.".format(weekday, date.dayOfMonth, date.monthValue)
        } else {
            "%s %d %s".format(weekday, date.dayOfMonth, date.month.getDisplayName(TextStyle.SHORT, I18n.locale()))
        }
        return if (text.length > 10) "$day ${text.substring(11)}" else day
    }

    /** A Forecast section's heading (ADR 0045). */
    fun forecastDay(d: Day): String = when (d.kind) {
        "overdue" -> tr("Überfällig")
        "flagged" -> tr("Markiert")
        "later" -> tr("Später")
        else -> if (d.today) tr("Heute") else day(d.date)
    }

    /** Why a row stands in Forecast, with the day where it names another. */
    fun why(row: TaskRow, section: Day?): String? {
        val word = when (row.why) {
            "due" -> tr("fällig")
            "planned" -> tr("geplant")
            "defer" -> tr("verfügbar ab")
            "tag" -> tr("Forecast-Tag")
            "done" -> tr("erledigt")
            else -> return null
        }
        val date = row.whyDate
        return if (date != null && date != section?.date) "$word ${day(date)}" else word
    }

    fun due(row: TaskRow): String? {
        val due = row.due ?: return null
        return when (row.dueState) {
            Due.OVERDUE -> tr("überfällig · {n}", "n" to day(due))
            Due.SOON -> tr("bald fällig · {n}", "n" to day(due))
            Due.NONE -> tr("fällig {n}", "n" to day(due))
        }
    }

    fun reason(reason: ReasonRow): String {
        val s = reason.subject.orEmpty()
        return when (reason.code) {
            "completed" -> tr("erledigt")
            "dropped" -> tr("verworfen")
            "deferred" -> tr("zurückgestellt bis {n}", "n" to day(s))
            "project_not_active" -> tr("Projekt ruht: {s}", "s" to s)
            "folder_dropped" -> tr("Ordner verworfen")
            "tag_not_active" -> s
            "waiting_in_sequence" -> tr("wartet: erst „{s}“", "s" to s)
            "has_open_children" -> tr("Gruppe: erst die Kinder")
            "container_unavailable" -> tr("Gruppe darüber nicht verfügbar")
            "blocked_by" -> tr("blockiert von „{s}“", "s" to s)
            "cycle" -> tr("Zyklus in den Daten")
            else -> tr("unbekannter Grund")
        }
    }

    /** The line under a row's title in the widget; the app's rows follow the layout (ADR 0130). */
    fun detail(row: TaskRow): String {
        val parts = mutableListOf<String>()
        if (row.state != null) parts += row.state!!.substringAfterLast('/')
        due(row)?.let { parts += it }
        if (!row.available && !row.done) row.reason?.let { parts += reason(it) }
        parts += row.tags.map { "@$it" }
        if (row.pending) parts += tr("nicht gesynct")
        return parts.joinToString(" · ")
    }

    fun chip(chip: Chip): String = when (chip.kind) {
        "template" -> tr("Vorlage: /{value}", "value" to chip.value)
        "project" -> tr("Projekt: {value}", "value" to chip.value)
        "tag" -> tr("Tag: {value}", "value" to chip.value)
        "new_tag" -> tr("Tag neu: {value}", "value" to chip.value)
        "flag" -> tr("markiert")
        "priority" -> tr("Prio {value}", "value" to chip.value)
        "defer" -> tr("Start: {n}", "n" to day(chip.value))
        "due" -> tr("Fällig: {n}", "n" to day(chip.value))
        "planned" -> tr("Geplant: {n}", "n" to day(chip.value))
        "estimate" -> tr("Schätzung: {n}", "n" to estimate(chip.value))
        "repeat" -> tr("Wiederholung: {value}", "value" to chip.value)
        "note" -> tr("Notiz")
        "property" -> "Property: ${chip.value}"
        else -> chip.value
    }

    /** "PT1H30M" → "1 h 30 min". */
    fun estimate(text: String): String {
        val m = Regex("""^PT(?:(\d+)H)?(?:(\d+)M)?$""").find(text) ?: return text
        val (h, min) = m.destructured
        return listOfNotNull(h.takeIf { it.isNotEmpty() }?.let { "$it h" },
            min.takeIf { it.isNotEmpty() }?.let { "$it min" }).joinToString(" ")
    }

    fun warning(w: WarningRow): String = when {
        w.code.startsWith("unknown_date") -> tr("„{text}“: kein Datum, das ich kenne; bleibt im Titel", "text" to w.text)
        w.code == "unknown_estimate" -> tr("„{text}“: keine Dauer; bleibt im Titel", "text" to w.text)
        w.code == "unknown_repeat" -> tr("„{text}“: keine Wiederholung, die ich kenne; bleibt im Titel", "text" to w.text)
        w.code.startsWith("twice") -> tr("„{text}“: doppelt; bleibt im Titel", "text" to w.text)
        w.code == "unknown_project" -> tr("#{text}: kein oder kein eindeutiges Projekt; geht in die Inbox", "text" to w.text)
        w.code == "unknown_tag" -> tr("@{text}: mehrdeutig oder neu unter einem Ablauf; bleibt im Titel", "text" to w.text)
        w.code == "unknown_property" -> tr("„{text}“: keine Property, die ich setzen kann (prop:key=wert, Key klein); bleibt im Titel", "text" to w.text)
        else -> w.text
    }

    /** "Mi 20.11. 08:30" from milliseconds, in the phone's zone. */
    fun instant(ms: Long): String {
        val at = Instant.ofEpochMilli(ms).atZone(ZoneId.systemDefault())
        return day(at.toLocalDate().toString()) + " %02d:%02d".format(at.hour, at.minute)
    }

    /** 30 → "30 min", 120 → "2 h", 1440 → "1 Tag". */
    fun minutes(m: Long): String = when {
        m % 1440 == 0L -> if (m == 1440L) tr("1 Tag") else tr("{n} Tage", "n" to m / 1440)
        m % 60 == 0L -> "${m / 60} h"
        else -> "$m min"
    }

    private fun anchor(a: Anchor): String = when (a) {
        Anchor.DUE -> tr("Fälligkeit")
        Anchor.PLANNED -> tr("Plan")
        Anchor.DEFER -> tr("Start")
        Anchor.AT, Anchor.DAILY, Anchor.UNKNOWN -> ""
    }

    /** A reminder on the task's page (ADR 0030). */
    fun reminder(r: ReminderRow): String {
        val what = when {
            r.anchor == Anchor.AT -> r.atMs?.let { instant(it) } ?: tr("Zeitpunkt")
            // ADR 0093: every day at its time; a habit only while it is open.
            r.anchor == Anchor.DAILY -> r.timeMinutes?.let { tr("täglich %d:%02d").format(it / 60, it % 60) } ?: tr("täglich")
            r.anchor == Anchor.UNKNOWN -> tr("unbekannte Erinnerung")
            r.beforeMinutes == null -> if (r.anchor == Anchor.DUE) tr("zur Fälligkeit") else tr("zum {n}", "n" to anchor(r.anchor))
            else -> tr("{n} vor {n2}", "n" to minutes(r.beforeMinutes!!), "n2" to anchor(r.anchor))
        }
        val off = r.goesOffMs?.takeIf { r.anchor != Anchor.AT }?.let { " · ${instant(it)}" } ?: ""
        val none = when {
            r.goesOffMs != null || r.anchor == Anchor.UNKNOWN -> ""
            r.anchor == Anchor.DAILY -> tr(" · diese Woche nicht")
            else -> tr(" · ohne Datum")
        }
        return what + off + none
    }

    /** 3900 seconds → "1:05". */
    fun length(seconds: Long): String {
        val minutes = maxOf(0L, seconds) / 60
        return "%d:%02d".format(minutes / 60, minutes % 60)
    }

    /** "Mo 09.11. 08:00 – 09:15", or "– läuft". */
    fun clockSpan(c: ClockRow): String {
        val end = c.endMs?.let {
            val at = Instant.ofEpochMilli(it).atZone(ZoneId.systemDefault())
            "%02d:%02d".format(at.hour, at.minute)
        } ?: tr("läuft")
        return instant(c.startMs) + " – " + end
    }

    /** The line under a notification's title. */
    fun reminded(n: Notice): String = tr("Erinnerung · {n}", "n" to instant(n.atMs))

    /** An appointment's notice (ADR 0101 §7): when and where. */
    fun appointmentNotice(a: AppointmentRow): String =
        listOfNotNull(CalendarApp.whenWords(a), a.location).joinToString(" · ")

    /**
     * One alarm of an appointment as the web says it (`alarmWords`,
     * ADR 0101 §9): "15 Min. vorher", "5 min nach dem Ende", "am Vortag
     * um 09:00", or the instant.
     */
    fun alarm(alarm: AlarmRow, a: AppointmentRow): String {
        alarm.atMs?.let { return instant(it) }
        val minutes = alarm.beforeMinutes ?: return "?"
        // Over whole days from midnight (§3): the day and the time it goes off.
        if (!alarm.end && a.allDay && minutes > 0 && minutes % 1440 != 0L) {
            val days = (minutes + 1439) / 1440
            val at = days * 1440 - minutes
            val time = "%02d:%02d".format(at / 60, at % 60)
            return if (days == 1L) tr("am Vortag um {time}", "time" to time) else tr("{days} Tage vorher um {time}", "days" to days, "time" to time)
        }
        if (minutes == 0L) return if (alarm.end) tr("zum Ende") else tr("zu Beginn")
        val words = alarmLength(kotlin.math.abs(minutes))
        return when {
            alarm.end -> if (minutes > 0) tr("{words} vor dem Ende", "words" to words) else tr("{words} nach dem Ende", "words" to words)
            else -> if (minutes > 0) tr("{words} vorher", "words" to words) else tr("{words} nach Beginn", "words" to words)
        }
    }

    /** As the web's `lengthWords`: "15 Min.", "1 Tag 2 h", "1 h 30 min". */
    private fun alarmLength(m: Long): String {
        val days = m / 1440
        val hours = (m % 1440) / 60
        val rest = m % 60
        if (days == 0L && hours == 0L) return tr("{rest} Min.", "rest" to rest)
        return listOfNotNull(
            if (days > 0) (if (days == 1L) tr("1 Tag") else tr("{n} Tage", "n" to days)) else null,
            if (hours > 0) "$hours h" else null,
            if (rest > 0) "$rest min" else null,
        ).joinToString(" ")
    }
}
