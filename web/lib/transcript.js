// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// What the inspector shows of an attachment's transcript (ADR 0140 §2,
// §3): the text, or where it stands, or the button that asks for it.
// No DOM here, so node can test it.

import { tr } from "./i18n.js";

// { text?, note?, ask? } for an audio attachment, null for anything else
// and for a recording nobody can ask about. `canAsk`: the server has a
// transcript service.
export function transcriptParts(a, canAsk) {
  if (!a.media_type.toLowerCase().startsWith("audio/")) return null;
  if (a.transcript_state === "pending") return { note: tr("Transkript läuft …") };
  if (a.transcript_state === "failed") {
    const note = `${tr("Kein Transkript:")} ${a.transcript_error ?? ""}`.trim();
    return canAsk ? { note, ask: tr("Erneut versuchen") } : { note };
  }
  if (typeof a.transcript === "string") {
    const text = a.transcript ? { text: a.transcript } : { note: tr("Nichts gesprochen.") };
    return canAsk ? { ...text, ask: tr("Neu transkribieren") } : text;
  }
  return canAsk ? { ask: tr("Transkribieren") } : null;
}
