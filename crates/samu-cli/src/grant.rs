// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu login --only`: the folders and projects a restricted session may
//! reach (ADR 0095 §8), named as in `samu folder list` and `samu projects`.

use anyhow::{Result, bail};
use samu_api::{AnyObject, ChangesPage, Folder, Id, Project};
use samu_client::{Client, Error as Failure};

use crate::short;

/// Whether every reference is an id already, so that no session is needed
/// to look them up.
pub(crate) fn ids_only(references: &[String]) -> Option<Vec<Id>> {
    references.iter().map(|r| as_id(r)).collect()
}

fn as_id(reference: &str) -> Option<Id> {
    reference
        .parse::<Id>()
        .ok()
        .or_else(|| Id::from_uri(reference))
}

/// The ids of the references, looked up through `client`, in their order,
/// each once.
pub(crate) fn resolve_all(client: &Client, references: &[String]) -> Result<Vec<Id>> {
    let (folders, projects) = places(client)?;
    let mut ids = Vec::new();
    for reference in references {
        let id = find(&folders, &projects, reference)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// The folders and projects the session sees.
fn places(client: &Client) -> Result<(Vec<Folder>, Vec<Project>)> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut folders = Vec::new();
    let mut projects = Vec::new();
    for object in page.objects {
        match object {
            AnyObject::Folder(f) => folders.push(f),
            AnyObject::Project(p) => projects.push(p),
            _ => {}
        }
    }
    Ok((folders, projects))
}

/// Titles of the granted objects for `samu session`: a folder by its path
/// as far as the session sees it, a project by its title, else the short
/// id.
pub(crate) fn titles(client: &Client, ids: &[Id]) -> Result<Vec<String>> {
    let (folders, projects) = places(client)?;
    Ok(ids
        .iter()
        .map(|id| {
            if folders.iter().any(|f| f.id == *id) {
                folder_path(&folders, *id)
            } else if let Some(p) = projects.iter().find(|p| p.id == *id) {
                p.title.clone()
            } else {
                short(id)
            }
        })
        .collect())
}

/// A folder's path from the top, `/` between, as `samu folder list` shows
/// it.
fn folder_path(folders: &[Folder], id: Id) -> String {
    let mut parts = Vec::new();
    let mut at = Some(id);
    while let Some(here) = at {
        let Some(folder) = folders.iter().find(|f| f.id == here) else {
            break;
        };
        // A cycle in broken data ends at the depth of the whole list.
        if parts.len() > folders.len() {
            break;
        }
        parts.push(folder.title.as_str());
        at = folder.parent;
    }
    parts.reverse();
    parts.join("/")
}

/// One folder or project: by id or `samu:` URI; a folder by its path
/// (`tec/ai`), a project by its folder's path and title (`tec/ai/samudo`);
/// then either by its title alone, then by the end of its id. Any case;
/// more than one match is an error.
fn find(folders: &[Folder], projects: &[Project], reference: &str) -> Result<Id> {
    if let Some(id) = as_id(reference) {
        return Ok(id);
    }
    let wanted = reference.trim().trim_matches('/').to_lowercase();
    let project_path = |p: &Project| match p.folder {
        Some(folder) => format!("{}/{}", folder_path(folders, folder), p.title),
        None => p.title.clone(),
    };
    let tries: [Vec<(Id, String)>; 3] = [
        folders
            .iter()
            .filter(|f| folder_path(folders, f.id).to_lowercase() == wanted)
            .map(|f| (f.id, folder_path(folders, f.id)))
            .chain(
                projects
                    .iter()
                    .filter(|p| project_path(p).to_lowercase() == wanted)
                    .map(|p| (p.id, project_path(p))),
            )
            .collect(),
        folders
            .iter()
            .filter(|f| f.title.to_lowercase() == wanted)
            .map(|f| (f.id, folder_path(folders, f.id)))
            .chain(
                projects
                    .iter()
                    .filter(|p| p.title.to_lowercase() == wanted)
                    .map(|p| (p.id, project_path(p))),
            )
            .collect(),
        folders
            .iter()
            .filter(|f| f.id.to_string().ends_with(&wanted))
            .map(|f| (f.id, folder_path(folders, f.id)))
            .chain(
                projects
                    .iter()
                    .filter(|p| p.id.to_string().ends_with(&wanted))
                    .map(|p| (p.id, project_path(p))),
            )
            .collect(),
    ];
    for found in tries {
        match found.as_slice() {
            [] => continue,
            [(id, _)] => return Ok(*id),
            many => bail!(
                "{reference:?} names {} folders and projects; use the path or the end of an id: {}",
                many.len(),
                many.iter()
                    .map(|(id, path)| format!("{path} ({})", short(id)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
    Err(Failure::NotFound(format!("no folder or project {reference:?}")).into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;
    use serde_json::json;

    fn folder(title: &str, parent: Option<Id>) -> Folder {
        let mut value = json!({
            "id": Id::new(),
            "created": "2026-11-13T10:00:00Z",
            "modified": "2026-11-13T10:00:00Z",
            "title": title,
            "status": "active",
            "position": 0,
        });
        if let Some(parent) = parent {
            value["parent"] = json!(parent);
        }
        serde_json::from_value(value).unwrap()
    }

    fn project(title: &str, folder: Option<Id>) -> Project {
        let mut value = json!({
            "id": Id::new(),
            "created": "2026-11-13T10:00:00Z",
            "modified": "2026-11-13T10:00:00Z",
            "title": title,
            "status": "active",
            "position": 0,
        });
        value["kind"] = json!("parallel");
        if let Some(folder) = folder {
            value["folder"] = json!(folder);
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_path_names_a_folder_or_a_project_below_one() {
        let tec = folder("tec", None);
        let ai = folder("ai", Some(tec.id));
        let other_ai = folder("AI", None);
        let samudo = project("samudo", Some(ai.id));
        let folders = vec![tec.clone(), ai.clone(), other_ai.clone()];
        let projects = vec![samudo.clone()];

        assert_eq!(find(&folders, &projects, "tec/ai").unwrap(), ai.id);
        assert_eq!(find(&folders, &projects, "Tec/AI/").unwrap(), ai.id);
        assert_eq!(find(&folders, &projects, "ai").unwrap(), other_ai.id);
        assert_eq!(
            find(&folders, &projects, "tec/ai/samudo").unwrap(),
            samudo.id
        );
        assert_eq!(find(&folders, &projects, "samudo").unwrap(), samudo.id);
        assert_eq!(find(&folders, &projects, &short(&tec.id)).unwrap(), tec.id);
        assert_eq!(
            find(&folders, &projects, &samudo.id.to_string()).unwrap(),
            samudo.id
        );
        let missing = find(&folders, &projects, "tec/nope").unwrap_err();
        assert!(matches!(
            missing.downcast_ref::<Failure>(),
            Some(Failure::NotFound(_))
        ));
    }

    #[test]
    fn a_title_two_share_is_an_error() {
        let a = folder("tec", None);
        let b = folder("priv", None);
        let one = project("misc", Some(a.id));
        let two = project("misc", Some(b.id));
        let error = find(&[a, b], &[one, two], "misc").unwrap_err().to_string();
        assert!(error.contains("tec/misc"), "{error}");
        assert!(error.contains("priv/misc"), "{error}");
    }

    #[test]
    fn ids_need_no_lookup() {
        let id = Id::new();
        assert_eq!(ids_only(&[id.to_string()]), Some(vec![id]));
        assert_eq!(ids_only(&[id.to_string(), "tec/ai".into()]), None);
    }
}
