#!/usr/bin/env python3
"""The screenshots of the web UI for the product page, from the public demo.

    site/screenshots.py de|en [--base https://demo.samudo.de] [--out DIR]

Signs in as demo-de or demo-en, sets the interface to that language and
takes the ten scenes of the gallery in www/content/<lang>/_index.md at
1440x900 with headless Chromium over CDP; the demo banner is hidden. The
images land in assets/screenshots/<lang>/ as 8-bit palette PNGs.

Needs /usr/bin/chromium and the Debian packages python3-websockets and
python3-pil. The demo resets on the hour: do not run it just before.
The demo password is public (deploy/demo/reset.sh); DEMO_PASSWORD
overrides it.
"""

import argparse
import asyncio
import base64
import io
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import time
import urllib.request

import websockets
from PIL import Image

PORT = 9333

# What differs per language: the names in the demo content and the labels
# the script clicks.
SCENES = {
    "de": {
        "project": "Hexeninventar",
        "task": "Oma Wetterwachs kauft",
        "perspective": "Schnelle Siege",
        "habits": "Gewohnheiten",
        "list": "Liste",
        "locale": "de-DE",
        "capture": "Besen zur Wartung bringen @Besorgung d:fr !A",
    },
    "en": {
        "project": "Witch inventory",
        "task": "Granny Weatherwax buys",
        "perspective": "Quick wins",
        "habits": "Habits",
        "list": "List",
        "locale": "en-US",
        "capture": "Take the broom in for service @Errand d:fri !A",
    },
}


def object_ids(base, account, password):
    """Title → id of every object of the account, from GET /changes."""
    req = urllib.request.Request(
        f"{base}/login",
        data=json.dumps({"account": account, "password": password, "device": "screenshots"}).encode(),
        headers={"content-type": "application/json"},
    )
    token = json.load(urllib.request.urlopen(req))["token"]
    auth = {"Authorization": f"Bearer {token}"}
    try:
        req = urllib.request.Request(f"{base}/changes?after=0", headers=auth)
        out = {}
        for x in json.load(urllib.request.urlopen(req))["objects"]:
            out.setdefault(x["object"].get("title"), x["object"]["id"])
        return out
    finally:
        urllib.request.urlopen(urllib.request.Request(f"{base}/logout", method="POST", headers=auth))


class Tab:
    def __init__(self, ws, base):
        self.ws, self.base, self.n = ws, base, 0

    async def call(self, method, **params):
        self.n += 1
        mid = self.n
        await self.ws.send(json.dumps({"id": mid, "method": method, "params": params}))
        while True:
            m = json.loads(await self.ws.recv())
            if m.get("id") == mid:
                if "error" in m:
                    raise RuntimeError(f"{method}: {m['error']}")
                return m.get("result", {})

    async def js(self, expr):
        r = await self.call("Runtime.evaluate", expression=expr, awaitPromise=True, returnByValue=True)
        if "exceptionDetails" in r:
            raise RuntimeError(r["exceptionDetails"])
        return r.get("result", {}).get("value")

    async def go(self, path):
        await self.js(f"location.href = {json.dumps(self.base + path)}")
        await asyncio.sleep(3)

    async def keys(self, *keys):
        for k in keys:
            await self.call("Input.dispatchKeyEvent", type="keyDown", key=k, text=k if len(k) == 1 else "")
            await self.call("Input.dispatchKeyEvent", type="keyUp", key=k)
            await asyncio.sleep(0.3)
        await asyncio.sleep(1.5)

    async def click(self, text, selector="*"):
        """Clicks the innermost visible element whose text contains `text`."""
        ok = await self.js(f"""(() => {{
          const els = [...document.querySelectorAll({json.dumps(selector)})].filter(e =>
            e.textContent.trim().includes({json.dumps(text)}) && e.offsetParent);
          if (!els.length) return false;
          els[els.length - 1].click();
          return true;
        }})()""")
        if not ok:
            raise RuntimeError(f"nothing to click with {text!r}")
        await asyncio.sleep(1.5)

    async def shot(self, out, name):
        await asyncio.sleep(0.5)
        data = base64.b64decode((await self.call("Page.captureScreenshot", format="png"))["data"])
        image = Image.open(io.BytesIO(data)).convert("RGB")
        image.quantize(256, method=Image.Quantize.MEDIANCUT).save(out / f"{name}.png", optimize=True)
        print(f"{name}.png")


async def take(base, lang, out, password):
    s = SCENES[lang]
    account = f"demo-{lang}"
    ids = object_ids(base, account, password)
    profile = tempfile.mkdtemp(prefix="samu-shots-")
    browser = subprocess.Popen(
        ["/usr/bin/chromium", "--headless=new", f"--user-data-dir={profile}", "--no-proxy-server",
         "--remote-allow-origins=*", f"--remote-debugging-port={PORT}", "--window-size=1440,900",
         "--hide-scrollbars", f"--lang={s['locale']}", "about:blank"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        for _ in range(50):
            try:
                version = json.load(urllib.request.urlopen(f"http://127.0.0.1:{PORT}/json/version"))
                break
            except OSError:
                time.sleep(0.2)
        else:
            raise RuntimeError("chromium did not start")
        # Page.navigate never returns here; a new target with the URL does.
        async with websockets.connect(version["webSocketDebuggerUrl"], proxy=None, max_size=None) as ws:
            target = (await Tab(ws, base).call("Target.createTarget", url=base + "/"))["targetId"]
        await asyncio.sleep(2)
        pages = json.load(urllib.request.urlopen(f"http://127.0.0.1:{PORT}/json"))
        url = next(p["webSocketDebuggerUrl"] for p in pages if p["id"] == target)
        async with websockets.connect(url, proxy=None, max_size=None) as ws:
            t = Tab(ws, base)
            await t.call("Page.enable")
            await t.call("Page.addScriptToEvaluateOnNewDocument", source="""
              document.addEventListener('DOMContentLoaded', () => {
                const s = document.createElement('style');
                s.textContent = '#demo-banner { display: none !important }';
                document.head.append(s);
              });""")
            # The native date fields stay mm/dd/yyyy in headless Chromium
            # whatever the locale (also with --lang or LANG); known, accepted.
            await t.call("Emulation.setLocaleOverride", locale=s["locale"])
            await t.call("Emulation.setDeviceMetricsOverride", width=1440, height=900,
                         deviceScaleFactor=1, mobile=False)
            login = json.dumps({"account": account, "password": password, "device": "web"})
            status = await t.js(f"""fetch('/login', {{method: 'POST',
              headers: {{'content-type': 'application/json'}}, body: {json.dumps(login)}}}).then(r => r.status)""")
            if status != 200:
                raise RuntimeError(f"sign-in answered {status}")
            await t.js(f"localStorage.setItem('samu.lang', '{lang}'); localStorage.setItem('samu.theme', 'light')")
            await t.go("/")

            await t.keys("g", "i")
            await t.shot(out, "inbox")
            await t.go("/o/" + ids[s["project"]])
            await t.click(s["task"])
            await t.shot(out, "project-detail")
            await t.click("Board", "button")
            await t.shot(out, "board")
            await t.click(s["list"], "button")
            await t.keys("Escape")
            await t.keys("g", "f")
            await t.shot(out, "forecast")
            await t.keys("g", "k")
            await t.shot(out, "calendar")
            await t.go("/o/" + ids[s["perspective"]])
            await t.shot(out, "perspective")
            await t.go("/o/" + ids[s["habits"]])
            await t.shot(out, "habits")
            await t.keys("g", "z")
            await t.shot(out, "report")
            await t.keys("g", "i")
            await t.keys("n")
            await t.call("Input.insertText", text=s["capture"])
            await asyncio.sleep(2)
            await t.shot(out, "capture")
            await t.keys("Escape")
            await t.js("localStorage.setItem('samu.theme', 'dark')")
            await t.go("/")
            await t.keys("g", "p")
            await t.shot(out, "dark")
            await t.js("fetch('/logout', {method: 'POST'}).then(r => r.status)")
    finally:
        browser.terminate()
        browser.wait()
        shutil.rmtree(profile, ignore_errors=True)


def main():
    here = pathlib.Path(__file__).resolve().parent
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("lang", choices=sorted(SCENES))
    p.add_argument("--base", default="https://demo.samudo.de")
    p.add_argument("--out", type=pathlib.Path)
    a = p.parse_args()
    out = a.out or here / "assets" / "screenshots" / a.lang
    out.mkdir(parents=True, exist_ok=True)
    asyncio.run(take(a.base.rstrip("/"), a.lang, out, os.environ.get("DEMO_PASSWORD", "demo4samudo")))


if __name__ == "__main__":
    main()
