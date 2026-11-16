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

## Updating Hextra

    site/hugo.sh mod get -u github.com/imfing/hextra && site/hugo.sh mod vendor
