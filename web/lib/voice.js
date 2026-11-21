// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// A voice note into the inbox with one button (ADR 0063 §1–§3): press to
// record, press again to stop; then a task „Sprachnotiz T.M. HH:MM“ goes
// in over POST /capture with the recording attached. Esc throws it away.
// The task is marked as a voice note and its attachment asks for the
// transcript, which then becomes title and note (ADR 0140 §4).

import { api, change } from "./api.js";
import { attachmentFor } from "./comments.js";
import { $ } from "./dom.js";
import { info } from "./message.js";
import { state, update } from "./state.js";
import { tr } from "./i18n.js";

// { recorder, stream, started, timer, keep } while recording.
let recording = null;

export const recordingVoice = () => recording !== null;

const two = (n) => String(n).padStart(2, "0");

// The title of the note, in the browser's local time.
export function voiceTitle(at) {
  return `${tr("Sprachnotiz")} ${at.getDate()}.${at.getMonth() + 1}. ${two(at.getHours())}:${two(at.getMinutes())}`;
}

function drawButton() {
  const button = $("voice");
  if (!button) return;
  button.classList.toggle("recording", recording !== null);
  button.setAttribute("aria-pressed", String(recording !== null));
  if (recording) {
    const seconds = Math.floor((Date.now() - recording.started) / 1000);
    button.firstChild.textContent = `■ ${Math.floor(seconds / 60)}:${two(seconds % 60)} `;
  } else {
    button.firstChild.textContent = `${tr("● Sprachnotiz")} `;
  }
}

export async function toggleVoice() {
  if (recording) {
    recording.recorder.stop();
    return;
  }
  if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
    update({ message: tr("Dieser Browser nimmt nicht auf.") });
    return;
  }
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (error) {
    update({ message: `${tr("Kein Mikrofon:")} ${error.message}` });
    return;
  }
  const type = MediaRecorder.isTypeSupported?.("audio/webm") ? "audio/webm" : "";
  const recorder = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
  const chunks = [];
  const started = new Date();
  recorder.addEventListener("dataavailable", (event) => event.data.size && chunks.push(event.data));
  recorder.addEventListener("stop", () => {
    for (const track of stream.getTracks()) track.stop();
    clearInterval(recording.timer);
    const keep = recording.keep;
    recording = null;
    drawButton();
    if (keep) save(new Blob(chunks, { type: recorder.mimeType || "audio/webm" }), started);
  });
  recording = { recorder, started: started.getTime(), timer: setInterval(drawButton, 1000), keep: true };
  recorder.start();
  drawButton();
}

// Esc while recording: stop and keep nothing.
export function cancelVoice() {
  if (!recording) return false;
  recording.keep = false;
  recording.recorder.stop();
  update({ message: info(tr("Sprachnotiz verworfen.")) });
  return true;
}

// The file goes up first, then the task, then the attachment (§2).
async function save(blob, started) {
  const at = `${started.getFullYear()}${two(started.getMonth() + 1)}${two(started.getDate())}-${two(started.getHours())}${two(started.getMinutes())}`;
  const file = new File([blob], `Sprachnotiz-${at}.webm`, { type: blob.type });
  try {
    const attachment = await attachmentFor(null, file);
    const answer = await api.capture(voiceTitle(started), null, false, state.scope);
    await api.write([
      change.set(answer.task.id, "properties.source", "voice"),
      change.create("attachment", { ...attachment, owner: answer.task.id, transcript_state: "pending" }),
    ]);
    update({ message: info(tr("„{title}“ angelegt.", { title: answer.task.title })) });
  } catch (error) {
    update({ message: `${tr("Sprachnotiz nicht gespeichert:")} ${error.message}` });
  }
}

export function wireVoice() {
  $("voice").addEventListener("click", toggleVoice);
}
