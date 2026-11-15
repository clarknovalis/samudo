# Documentation site

The documentation of samudo at <https://docs.samudo.de>: [Hugo](https://gohugo.io)
with the [Hextra](https://imfing.github.io/hextra/) theme, German at `/`,
English under `/en/`.

The pages themselves are not here. They are the Markdown in `docs/`
(English) and `docs/de/` (German), which stays readable in the repository;
this directory turns them into a site.

## Layout

- `hugo.yaml` – languages, menu, and the mounts: `../docs` and
  `../docs/de` become `assets/docs/<lang>/`, logo and favicon come from
  `../web/brand`.
- `adapter/_content.gotmpl` – a content adapter, mounted into both
  languages: it makes one page per Markdown file, its first `# Heading` the
  title, in the groups of `docs/README.md` (Start here, Clients, Features,
  Running, Developing). **A new page in `docs/` appears only once it is
  added to a group here**; a missing file in either language is a build
  warning.
- `layouts/_markup/render-link.html` – resolves the repository links
  between the pages (`concepts.md#scopes`) by file name; links that leave
  `docs/` stay text.
- `content/<lang>/` – what is not a doc page: the home page, a table of
  contents that points to samudo.de for what samudo is.
- `assets/screenshots/<lang>/` – the screenshots, made by
  `screenshots.py`. They are shown on the product page (`www/`), which
  mounts this directory; the documentation itself has none.
- `i18n/`, `assets/css/custom.css`, `layouts/_partials/` – labels, CSS
  tweaks, favicons.
- `_vendor/` – Hextra, checked in, so the build needs no network.

## Local preview

    site/hugo.sh server          # → http://localhost:1313

`hugo.sh` runs Hugo from the official image with the whole repository
mounted, so nothing needs installing. A build into `site/public/` (ignored
by git):

    site/hugo.sh --minify --gc -d public

## Rolling out

`site/public/` is a static site; any web server can serve it. How
docs.samudo.de is rolled out is not part of this repository.

## Screenshots

    site/screenshots.py de
    site/screenshots.py en

signs in to the public demo (<https://demo.samudo.de>) as `demo-de` or
`demo-en`, sets the interface to that language, hides the demo banner and
takes the ten scenes of the gallery in `www/content/<lang>/_index.md` at 1440×900 with
headless Chromium. The images replace those in
`assets/screenshots/<lang>/`; look at them before committing.

- Needs `/usr/bin/chromium` and the Debian packages `python3-websockets`
  and `python3-pil`.
- The demo resets on the hour: do not start it just before.
- The demo only shows what its image contains: after a change to the web
  UI, roll out the demo first.
- The native date fields stay `mm/dd/yyyy` in headless Chromium, in
  German too.
- A new scene needs a step in `take()`, an entry in both
  `www/content/<lang>/_index.md`, and, if it names something of the demo content, its
  title per language in `SCENES`.

### App screenshots

    site/app-screenshots.py de --serial <host:port>
    site/app-screenshots.py en --serial <host:port>

drives the app on a phone over adb and takes the eight scenes of
`SCENES` into `assets/screenshots/<lang>/app-*.png`, without status and
navigation bar. Before each run: phone unlocked, the app on `demo-de`
or `demo-en` and its language set to match (the choice is per account).
The account switch is no scene, since the menu lists every account on
the phone. Four of them are on the product page (`www/`).

## Updating Hextra

    site/hugo.sh mod get -u github.com/imfing/hextra && site/hugo.sh mod vendor
