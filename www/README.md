# www – the product page

The product page of samudo at <https://samudo.de> (ADR 0109): one page per
language, German at `/`, English at `/en/`. Everything beyond that belongs
in the documentation (`site/`, <https://docs.samudo.de>).

    www/hugo.sh server           # → http://localhost:1314
    www/hugo.sh --minify --gc    # → www/public/

Nothing is copied here:

- **Hextra** comes from the vendored copy in `site/_vendor`
  (`module.replacements` in `hugo.yaml`), so both pages use the same
  version. Updating it in `site/` updates it here.
- **Screenshots** come from `site/assets/screenshots/<lang>/`, made by
  `site/screenshots.py` (web) and `site/app-screenshots.py` (app).
- **Logo and favicon** come from `web/brand` and `web/favicon.svg`.

The text is in `content/de/_index.md` and `content/en/_index.md`, the
few styles of its own in `assets/css/custom.css`. No tracking, no cookies,
no external fonts or scripts.
