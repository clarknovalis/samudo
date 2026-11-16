// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Pairing a phone by QR code (ADR 0031, as catastr's web/lib/pair.js).
//
// Asks for the phone's name and the password once more — a stolen session
// alone must not let a phone in. The server answers with a code that works
// once and for ten minutes; this draws it for the app's scanner, with the
// code beneath for `samu login --code`.

import { api } from "./api.js";
import { el, replace } from "./dom.js";
import qrcode from "./qrcode-generator-2.0.4.js";
import { tr } from "./i18n.js";

export function openPairing() {
  const device = el("input", {
    id: "pair-device",
    autocomplete: "off",
    autocapitalize: "off",
    spellcheck: "false",
    required: true,
    placeholder: tr("z. B. handy"),
  });
  const password = el("input", {
    id: "pair-password",
    type: "password",
    autocomplete: "current-password",
    required: true,
  });
  const note = el("p", { class: "quiet", "aria-live": "polite" });
  const submit = el("button", { type: "submit", class: "primary", text: tr("Code erzeugen") });
  const form = el("form", { class: "pair-form" }, [
    el("label", { for: "pair-device", text: tr("Name des Handys") }),
    device,
    el("label", { for: "pair-password", text: tr("Dein Passwort, noch einmal") }),
    password,
    note,
    submit,
  ]);
  const result = el("div", { class: "pair-result", hidden: true });
  let timer = null;
  const close = () => {
    clearInterval(timer);
    document.removeEventListener("keydown", onKey);
    overlay.remove();
  };
  const onKey = (event) => {
    if (event.key === "Escape") close();
  };
  const box = el("div", { class: "box pair", role: "dialog", "aria-modal": "true", "aria-label": tr("Handy koppeln") }, [
    el("h2", { text: tr("Handy koppeln") }),
    form,
    result,
    el("button", { type: "button", text: tr("Schließen"), onclick: close }),
  ]);
  const overlay = el("div", { class: "overlay" }, [box]);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) close();
  });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    note.textContent = "";
    submit.disabled = true;
    let code;
    try {
      code = await api.pairCode(device.value.trim(), password.value);
    } catch (error) {
      note.textContent = error.status === 401 ? tr("Das Passwort stimmt nicht.") : error.message || tr("Kein Code");
      submit.disabled = false;
      return;
    } finally {
      password.value = "";
    }
    form.hidden = true;
    const expiry = el("p", { class: "quiet" });
    const until = new Date(code.expires);
    const tick = () => {
      const left = Math.max(0, Math.round((until - Date.now()) / 1000));
      if (left === 0) {
        clearInterval(timer);
        expiry.textContent = tr("Abgelaufen. Schließen und neu erzeugen.");
        result.classList.add("expired");
        return;
      }
      const minutes = Math.floor(left / 60);
      const seconds = String(left % 60).padStart(2, "0");
      expiry.textContent = tr("Gilt einmal, noch {time}", { time: `${minutes}:${seconds}` });
    };
    tick();
    timer = setInterval(tick, 1000);
    replace(
      result,
      el("p", {
        text: tr("In der App auf „QR-Code scannen“ tippen. Das Handy meldet sich als „{device}“ bei {account} an.", { device: code.device, account: code.account }),
      }),
      drawn(code.uri),
      expiry,
      el("code", { class: "pair-code", text: code.code }),
    );
    result.hidden = false;
  });

  document.addEventListener("keydown", onKey);
  document.body.append(overlay);
  device.focus();
}

// Black on white whatever the theme, with the quiet zone a scanner needs.
function drawn(text) {
  const qr = qrcode(0, "M");
  qr.addData(text);
  qr.make();
  const modules = qr.getModuleCount();
  const quiet = 4;
  const scale = 6;
  const side = (modules + 2 * quiet) * scale;
  const canvas = el("canvas", {
    class: "pair-qr",
    width: side,
    height: side,
    role: "img",
    "aria-label": tr("QR-Code zum Koppeln"),
  });
  const context = canvas.getContext("2d");
  context.fillStyle = "#fff";
  context.fillRect(0, 0, side, side);
  context.fillStyle = "#000";
  for (let row = 0; row < modules; row += 1) {
    for (let column = 0; column < modules; column += 1) {
      if (qr.isDark(row, column)) {
        context.fillRect((column + quiet) * scale, (row + quiet) * scale, scale, scale);
      }
    }
  }
  return canvas;
}
