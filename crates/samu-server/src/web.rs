// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The web UI, carried inside the binary (ADR 0025 §1, §2).
//!
//! Embedded rather than served from a directory, as in catastr: nothing to
//! mount in the container, nothing to get out of step with the API it talks
//! to. Matched by name, so an unknown name is a 404 and not a path that
//! reaches somewhere it should not. Served without a session: without one
//! the page shows the sign-in; the API stays behind the session.

use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// Every file of the UI: its path under the root, its content and type.
pub(crate) const FILES: &[(&str, &str, &str)] = &[
    ("index.html", include_str!("../../../web/index.html"), HTML),
    ("style.css", include_str!("../../../web/style.css"), CSS),
    ("app.js", include_str!("../../../web/app.js"), JS),
    ("favicon.svg", include_str!("../../../web/favicon.svg"), SVG),
    (
        "lib/address.js",
        include_str!("../../../web/lib/address.js"),
        JS,
    ),
    (
        "lib/account.js",
        include_str!("../../../web/lib/account.js"),
        JS,
    ),
    ("lib/api.js", include_str!("../../../web/lib/api.js"), JS),
    (
        "lib/blockdialog.js",
        include_str!("../../../web/lib/blockdialog.js"),
        JS,
    ),
    (
        "lib/blocks.js",
        include_str!("../../../web/lib/blocks.js"),
        JS,
    ),
    (
        "lib/board.js",
        include_str!("../../../web/lib/board.js"),
        JS,
    ),
    ("lib/bulk.js", include_str!("../../../web/lib/bulk.js"), JS),
    (
        "lib/columns.js",
        include_str!("../../../web/lib/columns.js"),
        JS,
    ),
    (
        "lib/close.js",
        include_str!("../../../web/lib/close.js"),
        JS,
    ),
    (
        "lib/closeask.js",
        include_str!("../../../web/lib/closeask.js"),
        JS,
    ),
    (
        "lib/colors.js",
        include_str!("../../../web/lib/colors.js"),
        JS,
    ),
    (
        "lib/palette.js",
        include_str!("../../../web/lib/palette.js"),
        JS,
    ),
    (
        "lib/capture.js",
        include_str!("../../../web/lib/capture.js"),
        JS,
    ),
    (
        "lib/offer.js",
        include_str!("../../../web/lib/offer.js"),
        JS,
    ),
    ("lib/edit.js", include_str!("../../../web/lib/edit.js"), JS),
    (
        "lib/habits.js",
        include_str!("../../../web/lib/habits.js"),
        JS,
    ),
    (
        "lib/detail.js",
        include_str!("../../../web/lib/detail.js"),
        JS,
    ),
    (
        "lib/clock.js",
        include_str!("../../../web/lib/clock.js"),
        JS,
    ),
    (
        "lib/clockedit.js",
        include_str!("../../../web/lib/clockedit.js"),
        JS,
    ),
    (
        "lib/logbook.js",
        include_str!("../../../web/lib/logbook.js"),
        JS,
    ),
    (
        "lib/author.js",
        include_str!("../../../web/lib/author.js"),
        JS,
    ),
    (
        "lib/report.js",
        include_str!("../../../web/lib/report.js"),
        JS,
    ),
    (
        "lib/voice.js",
        include_str!("../../../web/lib/voice.js"),
        JS,
    ),
    (
        "lib/comments.js",
        include_str!("../../../web/lib/comments.js"),
        JS,
    ),
    (
        "lib/alarms.js",
        include_str!("../../../web/lib/alarms.js"),
        JS,
    ),
    (
        "lib/appointment.js",
        include_str!("../../../web/lib/appointment.js"),
        JS,
    ),
    (
        "lib/calendar.js",
        include_str!("../../../web/lib/calendar.js"),
        JS,
    ),
    (
        "lib/calendars.js",
        include_str!("../../../web/lib/calendars.js"),
        JS,
    ),
    (
        "lib/addresses.js",
        include_str!("../../../web/lib/addresses.js"),
        JS,
    ),
    ("lib/dom.js", include_str!("../../../web/lib/dom.js"), JS),
    ("lib/demo.js", include_str!("../../../web/lib/demo.js"), JS),
    (
        "lib/templates.js",
        include_str!("../../../web/lib/templates.js"),
        JS,
    ),
    (
        "lib/layout.js",
        include_str!("../../../web/lib/layout.js"),
        JS,
    ),
    ("lib/menu.js", include_str!("../../../web/lib/menu.js"), JS),
    ("lib/pair.js", include_str!("../../../web/lib/pair.js"), JS),
    (
        "lib/perspectives.js",
        include_str!("../../../web/lib/perspectives.js"),
        JS,
    ),
    (
        "lib/places.js",
        include_str!("../../../web/lib/places.js"),
        JS,
    ),
    (
        "lib/placeops.js",
        include_str!("../../../web/lib/placeops.js"),
        JS,
    ),
    (
        "lib/scope.js",
        include_str!("../../../web/lib/scope.js"),
        JS,
    ),
    (
        "lib/chipfield.js",
        include_str!("../../../web/lib/chipfield.js"),
        JS,
    ),
    (
        "lib/settings.js",
        include_str!("../../../web/lib/settings.js"),
        JS,
    ),
    // The QR code for pairing (ADR 0031 §7): Kazuhiko Arase's generator,
    // MIT, the same file catastr carries; licence beside it in web/lib.
    (
        "lib/qrcode-generator-2.0.4.js",
        include_str!("../../../web/lib/qrcode-generator-2.0.4.js"),
        JS,
    ),
    ("lib/i18n.js", include_str!("../../../web/lib/i18n.js"), JS),
    ("lib/en.js", include_str!("../../../web/lib/en.js"), JS),
    (
        "lib/keycapture.js",
        include_str!("../../../web/lib/keycapture.js"),
        JS,
    ),
    ("lib/keys.js", include_str!("../../../web/lib/keys.js"), JS),
    (
        "lib/keymap.js",
        include_str!("../../../web/lib/keymap.js"),
        JS,
    ),
    (
        "lib/props.js",
        include_str!("../../../web/lib/props.js"),
        JS,
    ),
    (
        "lib/query.js",
        include_str!("../../../web/lib/query.js"),
        JS,
    ),
    (
        "lib/repeat.js",
        include_str!("../../../web/lib/repeat.js"),
        JS,
    ),
    (
        "lib/rowfields.js",
        include_str!("../../../web/lib/rowfields.js"),
        JS,
    ),
    (
        "lib/rowmenu.js",
        include_str!("../../../web/lib/rowmenu.js"),
        JS,
    ),
    (
        "lib/remind.js",
        include_str!("../../../web/lib/remind.js"),
        JS,
    ),
    (
        "lib/register.js",
        include_str!("../../../web/lib/register.js"),
        JS,
    ),
    (
        "lib/reorder.js",
        include_str!("../../../web/lib/reorder.js"),
        JS,
    ),
    (
        "lib/resize.js",
        include_str!("../../../web/lib/resize.js"),
        JS,
    ),
    ("lib/rich.js", include_str!("../../../web/lib/rich.js"), JS),
    (
        "lib/searchbar.js",
        include_str!("../../../web/lib/searchbar.js"),
        JS,
    ),
    ("lib/list.js", include_str!("../../../web/lib/list.js"), JS),
    ("lib/live.js", include_str!("../../../web/lib/live.js"), JS),
    (
        "lib/sidebar.js",
        include_str!("../../../web/lib/sidebar.js"),
        JS,
    ),
    (
        "lib/state.js",
        include_str!("../../../web/lib/state.js"),
        JS,
    ),
    (
        "lib/table.js",
        include_str!("../../../web/lib/table.js"),
        JS,
    ),
    (
        "lib/tabledata.js",
        include_str!("../../../web/lib/tabledata.js"),
        JS,
    ),
    (
        "lib/words.js",
        include_str!("../../../web/lib/words.js"),
        JS,
    ),
];

const HTML: &str = "text/html; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";
const SVG: &str = "image/svg+xml";

fn serve(path: &str) -> Response {
    let Some((_, body, kind)) = FILES.iter().find(|(name, _, _)| *name == path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = (*body).into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(kind));
    // "Ask me first", not "do not store": after an update the browser must
    // not mix a new index.html with an old app.js (as catastr found).
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

/// The page; on a demo with `<meta name="samu-demo">`, which shows its
/// banner (ADR 0098 §2).
fn page(state: &crate::app::AppState) -> Response {
    let Some(minutes) = state.demo_reset() else {
        return serve("index.html");
    };
    let Some((_, body, kind)) = FILES.iter().find(|(name, _, _)| *name == "index.html") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let body = demo_page(body, minutes);
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(kind));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

/// The page with the meta of a demo reset every `minutes`.
fn demo_page(page: &str, minutes: u32) -> String {
    page.replacen(
        "</head>",
        &format!("  <meta name=\"samu-demo\" content=\"{minutes}\">\n</head>"),
        1,
    )
}

/// `GET /` at the replica: never a demo.
pub(crate) async fn index() -> Response {
    serve("index.html")
}

/// `GET /` at the server.
pub(crate) async fn server_index(State(state): State<Arc<crate::app::AppState>>) -> Response {
    page(&state)
}

/// `GET /o/{id}`: the same page, opened on one object (ADR 0075 §1); the
/// page reads the id from its address. What is not an id is not a page.
pub(crate) async fn object_page(UrlPath(id): UrlPath<String>) -> Response {
    if id.parse::<samu_store::core::api::Id>().is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }
    serve("index.html")
}

/// The same at the server, with the banner of a demo.
pub(crate) async fn server_object_page(
    State(state): State<Arc<crate::app::AppState>>,
    UrlPath(id): UrlPath<String>,
) -> Response {
    if id.parse::<samu_store::core::api::Id>().is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }
    page(&state)
}

/// `GET /app.js`, `GET /style.css`, `GET /favicon.svg`
pub(crate) async fn root_file(UrlPath(name): UrlPath<String>) -> Response {
    serve(&name)
}

/// `GET /lib/{name}`
pub(crate) async fn lib_file(UrlPath(name): UrlPath<String>) -> Response {
    serve(&format!("lib/{name}"))
}

#[cfg(test)]
mod tests {
    //! What no compiler checks: the references the scripts make to the page,
    //! to each other and to the API (catastr's `tests/web_ui.rs`). Every
    //! module is read, not just the entry point.

    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use std::collections::BTreeSet;

    use samu_store::core::api::ROUTES;

    use super::FILES;

    fn scripts() -> impl Iterator<Item = (&'static str, &'static str)> {
        FILES
            .iter()
            .filter(|(name, _, _)| name.ends_with(".js"))
            .map(|(name, body, _)| (*name, *body))
    }

    fn index() -> &'static str {
        FILES[0].1
    }

    /// The text between `open` and the next `close`, everywhere in `text`.
    fn between<'a>(text: &'a str, open: &str, close: char) -> Vec<&'a str> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find(open) {
            rest = &rest[at + open.len()..];
            if let Some(end) = rest.find(close) {
                found.push(&rest[..end]);
                rest = &rest[end..];
            }
        }
        found
    }

    #[test]
    fn every_element_the_scripts_use_exists() {
        // Elements a script builds itself, `el("input", { id: "…" })`.
        let built: BTreeSet<&str> = scripts()
            .flat_map(|(_, body)| between(body, "id: \"", '"'))
            .collect();
        let mut checked = 0;
        for (name, body) in scripts() {
            for id in between(body, "$(\"", '"') {
                assert!(
                    index().contains(&format!("id=\"{id}\"")) || built.contains(id),
                    "{name} uses #{id}, which neither index.html nor a script makes"
                );
                checked += 1;
            }
        }
        assert!(
            checked > 20,
            "only {checked} ids found: the probe is broken"
        );
    }

    #[test]
    fn every_imported_module_is_served() {
        let served: BTreeSet<&str> = FILES.iter().map(|(name, _, _)| *name).collect();
        let mut checked = 0;
        for (name, body) in scripts() {
            let dir = name.rsplit_once('/').map_or("", |(dir, _)| dir);
            for spec in between(body, "from \"", '"') {
                let path = spec.strip_prefix("./").expect("relative import");
                let full = if dir.is_empty() {
                    path.to_string()
                } else {
                    format!("{dir}/{path}")
                };
                assert!(
                    served.contains(full.as_str()),
                    "{name} imports {spec}, not served"
                );
                checked += 1;
            }
        }
        assert!(checked > 20, "only {checked} imports found");
        // The page loads what it names, from the root so that it loads
        // under `/o/<uuid>` too (ADR 0075 §1); `#…` points into the page
        // itself (the logo's `<use href="#samudo-logo">`, ADR 0050). A link
        // (`<a href>`, the documentation in the demo banner, ADR 0098) loads
        // nothing.
        let links = between(index(), "<a href=\"", '"');
        for asset in between(index(), "src=\"", '"')
            .into_iter()
            .chain(between(index(), "href=\"", '"'))
            .filter(|a| !a.starts_with('#') && !links.contains(a))
        {
            let Some(file) = asset.strip_prefix('/') else {
                panic!("index.html loads {asset}, not from the root");
            };
            assert!(
                served.contains(file),
                "index.html loads {asset}, not served"
            );
        }
    }

    /// A path the UI calls, `${…}` read as a route parameter and the query
    /// cut off.
    fn route_of(literal: &str) -> String {
        let path = literal.split('?').next().unwrap_or(literal);
        let mut out = String::new();
        let mut rest = path;
        while let Some(at) = rest.find("${") {
            out.push_str(&rest[..at]);
            out.push_str("{id}");
            rest = rest[at..].split_once('}').map_or("", |(_, r)| r);
        }
        out.push_str(rest);
        out
    }

    #[test]
    fn every_route_the_ui_calls_is_served() {
        // A parameter is `{id}` on both sides, whatever the route calls it.
        let routes: BTreeSet<String> = ROUTES
            .iter()
            .map(|r| route_of(&r.path.replace('{', "${")))
            .collect();
        let mut checked = 0;
        for (name, body) in scripts() {
            // A text for the page (ADR 0105) may begin with a slash.
            let body = &body.replace("tr(\"/", "tr(\"");
            let literals = between(body, "(\"/", '"')
                .into_iter()
                .chain(between(body, "(`/", '`'));
            for literal in literals {
                let route = route_of(&format!("/{literal}"));
                assert!(
                    routes.contains(&route),
                    "{name} calls {route}, which is not a route"
                );
                checked += 1;
            }
        }
        assert!(checked >= 8, "only {checked} calls found");
    }

    /// A file under `web/` that is not embedded is a 404 in the browser and
    /// breaks the module that imports it; every one must be in `FILES`.
    #[test]
    fn every_file_of_the_ui_is_embedded() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web");
        let mut found = 0;
        for dir in ["", "lib"] {
            for entry in std::fs::read_dir(root.join(dir)).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().into_string().unwrap();
                let served = [".js", ".css", ".html", ".svg"]
                    .iter()
                    .any(|e| name.ends_with(e));
                if !entry.file_type().unwrap().is_file() || !served {
                    continue;
                }
                let path = if dir.is_empty() {
                    name
                } else {
                    format!("{dir}/{name}")
                };
                assert!(
                    FILES.iter().any(|(n, _, _)| *n == path),
                    "web/{path} is not embedded"
                );
                found += 1;
            }
        }
        assert!(found >= 20, "only {found} files looked at");
    }

    #[test]
    fn the_ui_calls_only_relative_paths() {
        // The vendored QR generator and lib/dom.js (the icons, ADR 0055 §7)
        // name the SVG namespace, no address they call; lib/addresses.js
        // holds link templates and examples the settings show, none of them
        // called.
        for (name, body) in scripts().filter(|(n, _)| {
            !n.starts_with("lib/qrcode-generator") && *n != "lib/addresses.js" && *n != "lib/dom.js"
        }) {
            for quote in ["\"http", "`http", "'http"] {
                assert!(!body.contains(quote), "{name} names an absolute address");
            }
        }
    }

    #[test]
    fn a_demo_page_carries_its_meta_once_in_the_head() {
        let page = super::demo_page(index(), 60);
        let meta = "<meta name=\"samu-demo\" content=\"60\">";
        assert_eq!(page.matches(meta).count(), 1);
        assert!(page.find(meta) < page.find("</head>"));
        assert!(!index().contains("samu-demo\" content"));
        assert!(index().contains("id=\"demo-banner\""));
    }

    #[test]
    fn the_probes_find_what_they_must() {
        assert_eq!(
            route_of("/objects/${encodeURIComponent(id)}/logbook"),
            "/objects/{id}/logbook"
        );
        assert_eq!(route_of("/changes?after=0"), "/changes");
        assert_eq!(between("a $(\"x\") b $(\"y\")", "$(\"", '"'), ["x", "y"]);
    }
}
