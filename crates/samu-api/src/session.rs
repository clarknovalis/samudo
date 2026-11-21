// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Signing in, sessions and errors (ADR 0019).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// `POST /login`: one account, its password, and a name for the device the
/// session belongs to (it becomes the device in server-made stamps).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    pub account: String,
    pub password: String,
    pub device: String,
    /// Whether a person or an agent signs in (ADR 0039 §2); none = a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::AuthorKind>,
    /// Restricts the session to these folders and projects (ADR 0095 §1);
    /// none = the whole account. Only at sign-in; it never changes after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<Grant>,
}

/// What a restricted session may reach (ADR 0095 §1): folders and projects
/// of its account, with everything below them, read-write or read-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub objects: Vec<crate::Id>,
    pub access: Access,
}

/// How a restricted session may touch what its grant reaches (ADR 0095 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Read and write.
    Rw,
    /// Read only.
    Ro,
    /// A value from a newer samu (ADR 0015): read, never written; a server
    /// refuses it at sign-in.
    #[serde(other)]
    Unknown,
}

/// `POST /login` answers with this, and `GET /session` with it minus the
/// token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub account: String,
    pub device: String,
    /// Whether a person or an agent is behind the session (ADR 0039 §2); a
    /// server before it leaves it out, which means a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::AuthorKind>,
    pub expires: Timestamp,
    /// The bearer token; only in the answer to `POST /login`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The account's zone (IANA), which decides "today" (ADR 0007, ADR 0028
    /// §6); a client that computes availability itself needs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// How long before `due` a task counts as due soon, unless it has its own
    /// warning (ADR 0017 §2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_soon: Option<crate::Period>,
    /// When a reminder on a date without a time goes off, in `zone`
    /// (ADR 0030 §3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remind_time: Option<jiff::civil::Time>,
    /// Only from the replica on a computer (ADR 0066 §6): how far it is
    /// with the server. Never from the server itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replica: Option<ReplicaState>,
    /// The session's grant, if it is restricted (ADR 0095 §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<Grant>,
    /// For a registered demo account: the day it is deleted unless it is
    /// used before (ADR 0099 §4), in the account's zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demo_until: Option<jiff::civil::Date>,
    /// Whether the server has a transcript service (ADR 0140 §3); a client
    /// offers transcripts only then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub transcribe: bool,
}

/// One session of the account in `GET /sessions` (ADR 0095 §7), without
/// its token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    /// The start of the hex of the token's hash: what
    /// `POST /sessions/{id}/end` takes. It does not give the token away.
    pub id: String,
    pub device: String,
    pub kind: crate::AuthorKind,
    pub created: Timestamp,
    pub expires: Timestamp,
    /// The session's grant, if it is restricted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<Grant>,
    /// Whether it is the session that asks.
    #[serde(default)]
    pub current: bool,
}

/// How the replica on a computer stands with the server (ADR 0066 §6), for
/// the status bar: "Offline · 3 Änderungen im Ausgang".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaState {
    /// The server it mirrors, like `https://samu.example.org`.
    pub upstream: String,
    /// Whether its last try to reach the server worked.
    pub online: bool,
    /// Changes not yet confirmed by the server.
    pub outbox: u32,
    /// When the last sync went through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<Timestamp>,
    /// What the server refused last, or why the sync failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// What every route answers with when it fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: ErrorCode,
    pub message: String,
}

/// The kind of failure, for a client to act on without reading the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Not signed in, or the sign-in failed.
    Unauthorized,
    /// Signed in, but not allowed.
    Forbidden,
    NotFound,
    BadRequest,
    /// The server failed.
    Internal,
    /// Only the server can do this, and the replica on a computer cannot
    /// reach it now (ADR 0066 §5); 503.
    Offline,
    /// A code from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// One route of the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
}

/// Every route the server offers. The server checks it serves each one,
/// and the command line that it has a command for each one (ADR 0019 §7,
/// catastr ADR 0038).
pub const ROUTES: &[Route] = &[
    Route {
        method: "GET",
        path: "/health",
    },
    Route {
        method: "POST",
        path: "/login",
    },
    Route {
        method: "POST",
        path: "/logout",
    },
    Route {
        method: "GET",
        path: "/session",
    },
    Route {
        method: "POST",
        path: "/changes",
    },
    Route {
        method: "GET",
        path: "/changes",
    },
    Route {
        method: "GET",
        path: "/objects/{id}",
    },
    Route {
        method: "GET",
        path: "/objects/{id}/logbook",
    },
    Route {
        method: "GET",
        path: "/logbook",
    },
    Route {
        method: "POST",
        path: "/objects/{id}/delete",
    },
    Route {
        method: "POST",
        path: "/objects/{id}/to-project",
    },
    Route {
        method: "POST",
        path: "/objects/{id}/close",
    },
    Route {
        method: "GET",
        path: "/available",
    },
    Route {
        method: "GET",
        path: "/available/projects",
    },
    Route {
        method: "GET",
        path: "/available/folders",
    },
    Route {
        method: "GET",
        path: "/blocks",
    },
    Route {
        method: "GET",
        path: "/scopes/scheduled",
    },
    Route {
        method: "GET",
        path: "/forecast",
    },
    // ADR 0101 §5.
    Route {
        method: "GET",
        path: "/alarms",
    },
    Route {
        method: "GET",
        path: "/clocks/report",
    },
    Route {
        method: "GET",
        path: "/report",
    },
    Route {
        method: "GET",
        path: "/habits",
    },
    Route {
        method: "GET",
        path: "/habits/{id}",
    },
    Route {
        method: "POST",
        path: "/capture",
    },
    Route {
        method: "POST",
        path: "/capture/lines",
    },
    Route {
        method: "POST",
        path: "/render",
    },
    Route {
        method: "GET",
        path: "/events",
    },
    Route {
        method: "GET",
        path: "/export",
    },
    Route {
        method: "GET",
        path: "/hooks",
    },
    Route {
        method: "POST",
        path: "/hooks",
    },
    Route {
        method: "POST",
        path: "/hooks/{name}",
    },
    Route {
        method: "POST",
        path: "/pair/code",
    },
    Route {
        method: "POST",
        path: "/pair",
    },
    Route {
        method: "GET",
        path: "/register",
    },
    Route {
        method: "POST",
        path: "/register",
    },
    Route {
        method: "POST",
        path: "/account/delete",
    },
    Route {
        method: "POST",
        path: "/demo",
    },
    Route {
        method: "PUT",
        path: "/blobs/{sha256}",
    },
    Route {
        method: "GET",
        path: "/blobs/{sha256}",
    },
    // ADR 0047, 0048.
    Route {
        method: "PUT",
        path: "/calendars/{id}/source",
    },
    Route {
        method: "POST",
        path: "/calendars/{id}/refresh",
    },
    Route {
        method: "POST",
        path: "/attachments/{id}/transcribe",
    },
    Route {
        method: "GET",
        path: "/appointments",
    },
    Route {
        method: "POST",
        path: "/calendars/discover",
    },
    // ADR 0100 §1.
    Route {
        method: "POST",
        path: "/calendars/{id}/discover",
    },
    // ADR 0100 §4.
    Route {
        method: "POST",
        path: "/calendars/{id}/import",
    },
    // ADR 0068.
    Route {
        method: "POST",
        path: "/appointments/{id}/edit",
    },
    // ADR 0095 §7.
    Route {
        method: "GET",
        path: "/sessions",
    },
    Route {
        method: "POST",
        path: "/sessions/{id}/end",
    },
];
