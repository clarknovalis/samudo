// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

package de.samudo.app

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import de.samudo.app.mobile.Samu
import java.util.Locale

/**
 * The language of the app (ADR 0105): German is the key. [tr] looks the
 * German text up in [EN] when the app is English and falls back to German;
 * `{name}` is filled after the lookup. The choice is per phone, in the
 * app's preferences, not in an account's database (ADR 0105 §2, 0107 §1):
 * `system` follows the context's locale, which Android's own app language
 * (Android 13+, `locales_config.xml`) sets too; `de` and `en` hold. [lang]
 * is Compose state, so a change redraws at once.
 */
object I18n {
    val CHOICES = listOf("system" to "System", "de" to "Deutsch", "en" to "English")

    /** `de` or `en`, as chosen or as the phone says. */
    var lang by mutableStateOf("de")
        private set

    /** What is chosen: `system`, `de` or `en`. */
    var choice by mutableStateOf("system")
        private set

    private const val PREFS = "samu"
    private const val KEY = "lang"

    private var system = "de"

    /** Whenever an account's database opens: reads the choice, tells samu-mobile. */
    fun init(context: Context, samu: Samu) {
        system = context.resources.configuration.locales[0]?.language ?: "de"
        choice = prefs(context).getString(KEY, null)?.takeIf { it == "de" || it == "en" } ?: "system"
        apply(samu)
    }

    fun choose(context: Context, value: String) {
        prefs(context).edit().apply { if (value == "system") remove(KEY) else putString(KEY, value) }.apply()
        choice = value
        apply(Store.get(context))
    }

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun apply(samu: Samu) {
        lang = when (choice) {
            "de", "en" -> choice
            else -> if (system.startsWith("de")) "de" else "en"
        }
        samu.setLang(lang)
    }

    /**
     * A message of samu-mobile in the app's language. samu-mobile speaks
     * English, as the server does (ADR 0105); the ones met while typing
     * are in [MESSAGES], the others stay English.
     */
    fun message(reason: String): String {
        if (lang != "de") return reason
        for ((pattern, german) in MESSAGES) {
            val found = pattern.regex.matchEntire(reason) ?: continue
            var text = german
            pattern.names.forEachIndexed { i, name ->
                val value = found.groupValues[i + 1]
                text = text.replace("{$name}", if (name == "what") WHAT[value] ?: value else value)
            }
            return text
        }
        return reason
    }

    /** German as Germany writes it, else the phone's English, else British. */
    fun locale(): Locale {
        if (lang == "de") return Locale.GERMAN
        val phone = Locale.getDefault()
        return if (phone.language == "en") phone else Locale.UK
    }
}

/**
 * A key may begin with a context, `Suche|Verwerfen`, where one German word
 * wants two English ones; German shows it without.
 */
private val CONTEXT = Regex("^\\p{L}+\\|")

fun tr(key: String, vararg vars: Pair<String, Any?>): String {
    var text = (if (I18n.lang == "en") EN[key] else null) ?: key.replaceFirst(CONTEXT, "")
    for ((name, value) in vars) text = text.replace("{$name}", value.toString())
    return text
}

/**
 * What a subtask has that a checklist item cannot keep, as samu-core names
 * it (`checklist`), and its German: the `{what}` of that refusal.
 * `web/tests/i18n.mjs` checks that samu-core still says each.
 */
private val WHAT: Map<String, String> = mapOf(
    "a note" to "eine Notiz",
    "dates" to "Datumsangaben",
    "tags" to "Tags",
    "an estimate" to "eine Schätzung",
    "a flag" to "eine Markierung",
    "a priority" to "eine Priorität",
    "properties" to "Properties",
    "a checklist of its own" to "eine eigene Checkliste",
    "comments" to "Kommentare",
    "attachments" to "Anhänge",
    "clocked time" to "gebuchte Zeit",
    "reminders" to "Erinnerungen",
    "links" to "Links",
    "a task that waits for it" to "eine Aufgabe, die auf sie wartet",
)

/** An English message with `{name}` for a value, quoted or not, and those names. */
private class Said(english: String) {
    val names = Regex("\\{(\\w+)\\}").findAll(english).map { it.groupValues[1] }.toList()
    val regex = Regex(
        english.split(Regex("\\{\\w+\\}")).joinToString("\"?(.+?)\"?") { Regex.escape(it) },
    )
}

/**
 * The messages of samu-mobile one meets while typing (3bc7fb69), by their
 * English text, and their German. `web/tests/i18n.mjs` checks that
 * samu-mobile still says each English one, so a reworded message fails
 * `make check` instead of falling back to English unseen.
 */
private val MESSAGES: List<Pair<Said, String>> = listOf(
    "a perspective needs a name" to "Eine Perspektive braucht einen Namen.",
    "a perspective needs a chip or a word" to "Eine Perspektive braucht einen Chip oder ein Wort.",
    "there is a perspective {title} already" to "Eine Perspektive „{title}“ gibt es schon.",
    "an icon is one emoji, at most 32 bytes" to "Ein Symbol ist ein Emoji, höchstens 32 Bytes.",
    "a task needs a title" to "Eine Aufgabe braucht einen Titel.",
    "a shared task needs a title" to "Eine geteilte Aufgabe braucht einen Titel.",
    "a title, please" to "Bitte einen Titel.",
    "nothing to capture" to "Nichts zu erfassen.",
    "a comment needs a text" to "Ein Kommentar braucht einen Text.",
    "a checklist item needs a text" to "Ein Checkpunkt braucht einen Text.",
    "a checklist item is one line of text" to "Ein Checkpunkt ist eine Zeile Text.",
    "the task is not a subtask" to "Die Aufgabe ist keine Unteraufgabe.",
    "the task has subtasks of its own" to "Die Aufgabe hat eigene Unteraufgaben.",
    "the task is a habit" to "Die Aufgabe ist ein Habit.",
    "the task repeats" to "Die Aufgabe wiederholt sich.",
    "the task was dropped" to "Die Aufgabe wurde verworfen.",
    "the task has {what}, which a checklist item cannot keep" to "Ein Checkpunkt kann nicht behalten, was die Aufgabe hat: {what}.",
    "{name} is empty" to "{name} ist leer.",
    "{name} has {size} bytes; the server keeps at most {max}" to "{name} hat {size} Bytes; der Server nimmt höchstens {max}.",
    "a reminder comes before, not after" to "Eine Erinnerung kommt vorher, nicht danach.",
    "a time of day is between 00:00 and 23:59" to "Eine Uhrzeit liegt zwischen 00:00 und 23:59.",
    "an amount is at least 1" to "Eine Menge ist mindestens 1.",
    "{day} is after today; an entry is for today or a day before" to "{day} liegt nach heute; ein Eintrag gilt für heute oder einen Tag davor.",
    "the settings come with the next sync" to "Die Einstellungen kommen mit dem nächsten Sync.",
    "days: a number from 1 to 366" to "Tage: eine Zahl von 1 bis 366.",
    "{text} is no date" to "„{text}“ ist kein Datum.",
    "{value} is no priority: A, B or C" to "„{value}“ ist keine Priorität: A, B oder C.",
    "{days} days is no warning" to "{days} Tage sind keine Vorwarnung.",
    "{key} is no property key: lower-case letters, digits, _ and -, a letter first" to
        "„{key}“ ist kein Schlüssel: Kleinbuchstaben, Ziffern, _ und -, vorn ein Buchstabe.",
    "only a workflow can be the default workflow" to "Nur ein Ablauf kann der Standard-Ablauf sein.",
    "a project uses this workflow; give it another one first" to "Ein Projekt nutzt diesen Ablauf; gib ihm zuerst einen anderen.",
    "only the states of a workflow have a key and a note" to "Taste und Notiz gibt es nur bei Zuständen eines Ablaufs.",
    "a key is one character" to "Eine Taste ist ein Zeichen.",
    "another state of this workflow has this key already" to "Diese Taste hat schon ein anderer Zustand dieses Ablaufs.",
).map { (english, german) -> Said(english) to german }
