#!/usr/bin/env python3
"""The app screenshots of the documentation site, from a phone over adb.

    site/app-screenshots.py de|en [scene ...] [--serial HOST:PORT]

Drives the samudo app on a connected phone and takes the eight scenes of
SCENES for the account that is active in the app: demo-de for de, demo-en
for en, with the app language set to match (Settings > Sprache/Language;
the choice is per account). Status bar and navigation bar are cut off; the
images land in assets/screenshots/<lang>/app-<scene>.png as 8-bit palette
PNGs.

Before running: phone unlocked, the app open on the right account. Its
menu also lists every other account on the phone, so the account switch
itself is no scene. The capture scene types its line but never sends it.
Labels are found with `uiautomator dump`, so the app's own words are the
selectors; a label that changes needs its entry in LABELS.
"""

import argparse
import io
import pathlib
import re
import subprocess
import sys
import time

from PIL import Image

LABELS = {
    "de": dict(inbox="Inbox", available="Verfügbar", forecast="Forecast", projects="Projekte",
               search="Suche", project="Hexeninventar", list="Liste", perspective="Schnelle Siege",
               capture="Erfassen", text="Besen zur Wartung bringen @Besorgung d:fr !A"),
    "en": dict(inbox="Inbox", available="Available", forecast="Forecast", projects="Projects",
               search="Search", project="Witch inventory", list="List", perspective="Quick wins",
               capture="Capture", text="Take the broom in for service @Errand d:fri !A"),
}
SCENES = ["inbox", "available", "projects", "project", "forecast", "search", "perspective", "capture"]


class Phone:
    def __init__(self, serial):
        self.serial = serial

    def adb(self, *args):
        cmd = ["adb"] + (["-s", self.serial] if self.serial else []) + list(args)
        return subprocess.run(cmd, capture_output=True, check=True).stdout

    def nodes(self):
        """(label, bounds) of every node with a text or description."""
        for _ in range(6):  # the dump fails now and then while the UI moves
            self.adb("shell", "rm", "-f", "/sdcard/ui.xml")
            self.adb("shell", "uiautomator", "dump", "/sdcard/ui.xml")
            xml = self.adb("exec-out", "cat", "/sdcard/ui.xml").decode()
            found = []
            for m in re.finditer(r"<node [^>]*>", xml):
                n = m.group(0)
                label = re.search(r' text="([^"]*)"', n).group(1) or re.search(r'content-desc="([^"]*)"', n).group(1)
                bounds = [int(v) for v in re.findall(r"\d+", re.search(r'bounds="([^"]*)"', n).group(1))]
                if label:
                    found.append((label, bounds))
            if found:
                return found
            time.sleep(1)
        sys.exit("uiautomator found nothing: is the phone unlocked?")

    def tap(self, label):
        nodes = self.nodes()
        hits = [b for t, b in nodes if t == label] or [b for t, b in nodes if label in t]
        if not hits:
            sys.exit(f"not on screen: {label}")
        b = hits[0]
        self.adb("shell", "input", "tap", str((b[0] + b[2]) // 2), str((b[1] + b[3]) // 2))
        time.sleep(1.2)

    def key(self, name):
        self.adb("shell", "input", "keyevent", name)
        time.sleep(0.8)

    def type(self, text):
        self.adb("shell", "input", "text", "'" + text.replace(" ", "%s") + "'")

    def bars(self):
        """Height of the status bar and top of the navigation bar."""
        dump = self.adb("shell", "dumpsys", "window").decode()
        top = int(re.search(r"statusBars frame=\[0,0\]\[\d+,(\d+)\]", dump).group(1))
        bottom = int(re.search(r"navigationBars frame=\[0,(\d+)\]", dump).group(1))
        return top, bottom

    def shot(self, path, bars):
        time.sleep(1.5)
        image = Image.open(io.BytesIO(self.adb("exec-out", "screencap", "-p"))).convert("RGB")
        image = image.crop((0, bars[0], image.width, bars[1]))
        image.quantize(colors=256, method=Image.Quantize.MEDIANCUT).save(path, optimize=True)
        print(path, file=sys.stderr)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("lang", choices=LABELS)
    ap.add_argument("scenes", nargs="*", help="default: all of " + " ".join(SCENES))
    ap.add_argument("--serial", help="adb device, e.g. 192.168.0.10:5555")
    args = ap.parse_args()
    unknown = set(args.scenes) - set(SCENES)
    if unknown:
        ap.error("unknown scene: " + " ".join(sorted(unknown)))
    w = LABELS[args.lang]
    phone = Phone(args.serial)
    out = pathlib.Path(__file__).parent / "assets" / "screenshots" / args.lang
    bars = phone.bars()

    def shot(name):
        phone.shot(out / f"app-{name}.png", bars)

    steps = {
        "inbox": lambda: (phone.tap(w["inbox"]), shot("inbox")),
        "available": lambda: (phone.tap(w["available"]), shot("available")),
        "projects": lambda: (phone.tap(w["projects"]), shot("projects")),
        "project": lambda: (phone.tap(w["projects"]), phone.tap(w["project"]), shot("project")),
        "forecast": lambda: (phone.tap(w["forecast"]), phone.tap(w["list"]), shot("forecast")),
        # The search field takes the focus; Back closes the keyboard.
        "search": lambda: (phone.tap(w["search"]), phone.key("BACK"), shot("search")),
        "perspective": lambda: (phone.tap(w["search"]), phone.key("BACK"), phone.tap(w["perspective"]),
                                shot("perspective")),
        # Back once closes the keyboard, twice the sheet: nothing is sent.
        "capture": lambda: (phone.tap(w["inbox"]), phone.tap(w["capture"]), phone.type(w["text"]),
                            phone.key("BACK"), shot("capture"), phone.key("BACK")),
    }
    for name in args.scenes or SCENES:
        steps[name]()


if __name__ == "__main__":
    main()
