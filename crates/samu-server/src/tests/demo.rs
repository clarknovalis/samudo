//! The demo content (ADR 0099 §6): both languages into an empty account,
//! counted in the database, not in what the seeding says it made.

use std::collections::BTreeMap;

use jiff::civil::date;
use samu_store::{AccountStore, BlobDir};

use super::TempDir;
use crate::demo::{self, Lang};

/// Live objects per kind, asked of the account's database.
fn kinds(path: &std::path::Path) -> BTreeMap<String, i64> {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut statement = conn
        .prepare("SELECT kind, count(*) FROM object WHERE deleted_at IS NULL GROUP BY kind")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn seeded(lang: Lang) -> (TempDir, BTreeMap<String, i64>) {
    let dir = TempDir::new();
    let path = dir.0.join("demo.db");
    let mut store = AccountStore::open(&path).unwrap();
    demo::seed(&mut store, &BlobDir::beside(&path), lang).unwrap();
    drop(store);
    let kinds = kinds(&path);
    (dir, kinds)
}

/// What both languages hold: the same demo, translated.
fn expected() -> BTreeMap<String, i64> {
    [
        ("attachment", 1),
        // Six from `clocks`, one from `z:` in a line.
        ("clock", 7),
        ("comment", 8),
        ("folder", 5),
        // 74 from the histories, one pause.
        ("habit_entry", 75),
        ("link", 3),
        ("perspective", 7),
        ("project", 14),
        ("reminder", 5),
        ("scope", 3),
        ("settings", 1),
        ("tag", 31),
        ("task", 81),
        ("template", 3),
    ]
    .into_iter()
    .map(|(k, n)| (k.to_string(), n))
    .collect()
}

#[test]
fn german_goes_into_an_empty_account() {
    let (_dir, kinds) = seeded(Lang::De);
    assert_eq!(kinds, expected());
}

#[test]
fn english_goes_into_an_empty_account() {
    let (_dir, kinds) = seeded(Lang::En);
    assert_eq!(kinds, expected());
}

#[test]
fn an_account_with_content_is_refused_and_left_alone() {
    let (dir, before) = seeded(Lang::De);
    let path = dir.0.join("demo.db");
    let mut store = AccountStore::open(&path).unwrap();
    let error = demo::seed(&mut store, &BlobDir::beside(&path), Lang::En).unwrap_err();
    assert!(error.to_string().contains("not empty"), "{error}");
    drop(store);
    assert_eq!(kinds(&path), before);
}

#[test]
fn the_attachment_is_a_file_among_the_blobs() {
    let (dir, _) = seeded(Lang::En);
    let path = dir.0.join("demo.db");
    let store = AccountStore::open(&path).unwrap();
    let objects = store.snapshot().unwrap();
    let [attachment] = objects.attachments.as_slice() else {
        panic!("one attachment, not {:?}", objects.attachments);
    };
    let mut text = String::new();
    std::io::Read::read_to_string(
        &mut BlobDir::beside(&path)
            .open(&attachment.blob)
            .unwrap()
            .unwrap(),
        &mut text,
    )
    .unwrap();
    assert!(text.starts_with("# "), "{text}");
    assert_eq!(attachment.size, text.len() as u64);
    // The day in it is filled in.
    assert!(!text.contains("{+"), "{text}");
}

#[test]
fn days_are_counted_from_today() {
    let content = demo::with_days("a {+1} b {-2} {value} {x} {+0}", date(2026, 12, 31)).unwrap();
    assert_eq!(content, "a 2027-01-01 b 2026-12-29 {value} {x} 2026-12-31");
}

// --- POST /demo: into an account with content (ADR 0099 §7) ----------------

/// The whole German demo, then the demo again: every name of the second is
/// one the account has already, so nothing may be looked up by name alone.
#[test]
fn into_an_account_with_content_everything_lies_under_demo() {
    for lang in [Lang::De, Lang::En] {
        let (dir, _) = seeded(Lang::De);
        let path = dir.0.join("demo.db");
        let mut store = AccountStore::open(&path).unwrap();
        let before = store.snapshot().unwrap();

        let (whole, counts) = demo::seed_into(&mut store, &BlobDir::beside(&path), lang).unwrap();
        assert!(!whole);
        let after = store.snapshot().unwrap();

        // What was there is untouched.
        let unchanged = |name: &str, old: Vec<serde_json::Value>, new: &[serde_json::Value]| {
            for o in &old {
                assert!(new.contains(o), "{name} changed: {o}");
            }
        };
        unchanged("task", to_values(&before.tasks), &to_values(&after.tasks));
        unchanged(
            "project",
            to_values(&before.projects),
            &to_values(&after.projects),
        );
        unchanged(
            "folder",
            to_values(&before.folders),
            &to_values(&after.folders),
        );
        unchanged("tag", to_values(&before.tags), &to_values(&after.tags));
        assert_eq!(
            serde_json::to_value(&after.settings).unwrap(),
            serde_json::to_value(&before.settings).unwrap()
        );
        assert_eq!(after.perspectives.len(), before.perspectives.len());
        assert_eq!(after.scopes.len(), before.scopes.len());
        assert_eq!(after.templates.len(), before.templates.len());

        // The new tags under one new root `Demo`.
        let new_tags: Vec<_> = after
            .tags
            .iter()
            .filter(|t| !before.tags.iter().any(|b| b.id == t.id))
            .collect();
        let roots: Vec<_> = new_tags.iter().filter(|t| t.parent.is_none()).collect();
        assert_eq!(roots.len(), 1, "{roots:?}");
        assert_eq!(roots[0].title, "Demo");
        let root = roots[0].id;
        let under_root = |mut id: samu_store::core::api::Id| {
            for _ in 0..10 {
                if id == root {
                    return true;
                }
                match after
                    .tags
                    .iter()
                    .find(|t| t.id == id)
                    .and_then(|t| t.parent)
                {
                    Some(p) => id = p,
                    None => return false,
                }
            }
            false
        };
        assert!(new_tags.iter().all(|t| under_root(t.id)));
        assert_eq!(new_tags.len(), 32);

        // The new folders in the tree of a new `Demo` folder, the projects
        // in them, the tasks in those projects; the inbox gets nothing.
        let new_folders: Vec<_> = after
            .folders
            .iter()
            .filter(|f| !before.folders.iter().any(|b| b.id == f.id))
            .collect();
        let demo_folder = new_folders
            .iter()
            .find(|f| f.parent.is_none())
            .expect("a folder Demo");
        assert_eq!(demo_folder.title, "Demo");
        assert!(
            new_folders
                .iter()
                .all(|f| f.id == demo_folder.id || f.parent.is_some()),
            "only one new top folder"
        );
        let folder_ids: Vec<_> = new_folders.iter().map(|f| f.id).collect();
        assert!(
            new_folders
                .iter()
                .all(|f| f.parent.is_none_or(|p| folder_ids.contains(&p)))
        );
        assert_eq!(new_folders.len(), 6);
        let new_projects: Vec<_> = after
            .projects
            .iter()
            .filter(|p| !before.projects.iter().any(|b| b.id == p.id))
            .collect();
        assert_eq!(new_projects.len(), 15);
        assert!(
            new_projects
                .iter()
                .all(|p| p.folder.is_some_and(|f| folder_ids.contains(&f))),
            "a project outside Demo"
        );
        let inbox_name = if lang == Lang::De { "Eingang" } else { "Inbox" };
        assert!(
            new_projects
                .iter()
                .any(|p| p.title == inbox_name && p.folder == Some(demo_folder.id))
        );
        let project_ids: Vec<_> = new_projects.iter().map(|p| p.id).collect();
        let new_tasks: Vec<_> = after
            .tasks
            .iter()
            .filter(|t| !before.tasks.iter().any(|b| b.id == t.id))
            .collect();
        assert_eq!(new_tasks.len(), 81);
        assert!(
            new_tasks
                .iter()
                .all(|t| t.project.is_some_and(|p| project_ids.contains(&p))),
            "a task outside Demo"
        );
        // Tags on the new tasks are the new ones only.
        let tag_ids: Vec<_> = new_tags.iter().map(|t| t.id).collect();
        assert!(
            new_tasks
                .iter()
                .all(|t| t.tags.iter().all(|x| tag_ids.contains(&x.tag)))
        );
        assert_eq!(counts.0.get("tasks"), Some(&81));

        // Once is enough: a second time is a conflict and changes nothing.
        drop(after);
        let again = demo::seed_into(&mut store, &BlobDir::beside(&path), lang).unwrap_err();
        assert!(again.downcast_ref::<demo::Taken>().is_some(), "{again:#}");
        assert_eq!(
            store.snapshot().unwrap().tasks.len(),
            before.tasks.len() + 81
        );
    }
}

fn to_values<T: serde::Serialize>(items: &[T]) -> Vec<serde_json::Value> {
    items
        .iter()
        .map(|i| serde_json::to_value(i).unwrap())
        .collect()
}

#[test]
fn an_empty_account_gets_everything() {
    let dir = TempDir::new();
    let path = dir.0.join("demo.db");
    let mut store = AccountStore::open(&path).unwrap();
    let (whole, _) = demo::seed_into(&mut store, &BlobDir::beside(&path), Lang::En).unwrap();
    assert!(whole);
    drop(store);
    assert_eq!(kinds(&path), expected());
}

#[test]
fn the_tags_of_a_line_go_under_demo() {
    assert_eq!(
        demo::shield_tags(r#"a @Ort/Lancre b@c @"Person/Oma W" d:+1d @ x"#),
        r#"a @Demo/Ort/Lancre b@c @"Demo/Person/Oma W" d:+1d @ x"#
    );
    assert_eq!(demo::shield_tags("@Klacker"), "@Demo/Klacker");
}
