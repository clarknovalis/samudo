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
 * `{name}` is filled after the lookup. The choice is per phone, in its
 * cache: `system` follows the context's locale, which Android's own app
 * language (Android 13+, `locales_config.xml`) sets too; `de` and `en`
 * hold. [lang] is Compose state, so a change redraws at once.
 */
object I18n {
    val CHOICES = listOf("system" to "System", "de" to "Deutsch", "en" to "English")

    /** `de` or `en`, as chosen or as the phone says. */
    var lang by mutableStateOf("de")
        private set

    /** What is chosen: `system`, `de` or `en`. */
    var choice by mutableStateOf("system")
        private set

    private var system = "de"

    /** Once the database is open: reads the choice, tells samu-mobile. */
    fun init(context: Context, samu: Samu) {
        system = context.resources.configuration.locales[0]?.language ?: "de"
        choice = runCatching { samu.langChoice() }.getOrDefault("system")
        apply(samu)
    }

    fun choose(context: Context, value: String) {
        val samu = Store.get(context)
        runCatching { samu.setLangChoice(value) }.onSuccess {
            choice = value
            apply(samu)
        }
    }

    private fun apply(samu: Samu) {
        lang = when (choice) {
            "de", "en" -> choice
            else -> if (system.startsWith("de")) "de" else "en"
        }
        samu.setLang(lang)
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
