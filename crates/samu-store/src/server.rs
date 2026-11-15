//! The server file: accounts, sessions (ADR 0019 §2, §3) and hooks
//! (ADR 0022 §3).
//!
//! No `flock` here, unlike an account database: `samu-server account …` has
//! to write beside a running server, and SQLite with WAL and a busy timeout
//! takes two writers to a small file.

use std::path::Path;
use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use jiff::{Timestamp, ToSpan};
use rand::TryRngCore;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::core::api::{Access, AuthorKind, Grant, HookFormat, SessionSummary};
use crate::core::hook::validate_hook_name;
use crate::migrations::{self, MigrationReport};
use crate::store::{author_kind_name, author_kind_of};
use crate::{Error, Result, open};

/// How long a session lives after its last use (ADR 0019 §3).
const SESSION_DAYS: i64 = 30;

/// Shortest password taken, as in catastr.
const MIN_PASSWORD_CHARS: usize = 8;

const TOKEN_BYTES: usize = 32;

/// How long a pairing code is worth anything (ADR 0031 §2).
const PAIRING_MINUTES: i64 = 10;

/// 128 bits (ADR 0031 §2).
const PAIRING_BYTES: usize = 16;

/// A hash checked when the account does not exist, so an unknown name takes
/// as long as a wrong password (catastr ADR 0074 §2).
static DUMMY_HASH: OnceLock<Option<String>> = OnceLock::new();

/// One account, as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    pub name: String,
    pub created: Timestamp,
    pub disabled: bool,
    pub registered: Registered,
    /// The last sign-in, or `created` before the first (ADR 0099 §4).
    pub last_active: Timestamp,
}

/// How an account came to be (ADR 0099 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registered {
    /// `samu-server account add`.
    Cli,
    /// `POST /register` in the mode `demo`: deleted after a rest.
    Demo,
    /// `POST /register` in the mode `open`.
    Open,
}

impl Registered {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Demo => "demo",
            Self::Open => "open",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "cli" => Some(Self::Cli),
            "demo" => Some(Self::Demo),
            "open" => Some(Self::Open),
            _ => None,
        }
    }
}

/// How many hex digits of the token's hash name a session (ADR 0095 §7).
pub const SESSION_ID_LEN: usize = 12;

/// What became of `end_session`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Ended,
    /// No session of the account starts so.
    Unknown,
    /// More than one does.
    Ambiguous,
}

/// A live session, as the server sees it on a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub account: String,
    pub device: String,
    /// A person or an agent (ADR 0039 §2).
    pub kind: AuthorKind,
    pub expires: Timestamp,
    /// What a restricted session may reach; none = the whole account
    /// (ADR 0095 §1).
    pub grant: Option<Grant>,
}

/// A pairing code just made: the secret this once, and what it is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub code: String,
    pub account: String,
    pub device: String,
    pub expires: Timestamp,
}

/// A hook, as the server keeps it; the secret travels apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRow {
    pub name: String,
    /// The account whose inbox it fills.
    pub account: String,
    pub format: HookFormat,
    pub created: Timestamp,
}

fn format_name(format: HookFormat) -> Result<&'static str> {
    match format {
        HookFormat::IndexV1 => Ok("index-v1"),
        HookFormat::Unknown => Err(unsupported("a hook format this server does not know")),
    }
}

fn hook_row(name: String, account: String, format: &str, created: &str) -> Result<HookRow> {
    let format = match format {
        "index-v1" => HookFormat::IndexV1,
        other => {
            return Err(Error::Corrupt {
                what: format!("hook.format {other:?}"),
            });
        }
    };
    Ok(HookRow {
        name,
        account,
        format,
        created: created.parse().map_err(|e| Error::Corrupt {
            what: format!("hook.created: {e}"),
        })?,
    })
}

/// The server file, open.
#[derive(Debug)]
pub struct ServerStore {
    conn: Connection,
    migration: MigrationReport,
}

fn unsupported(what: impl Into<String>) -> Error {
    Error::Unsupported { what: what.into() }
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| unsupported(format!("no random bytes: {e}")))?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A grant as it is kept, after checking it names something and an access
/// this server knows (ADR 0095 §1).
fn grant_json(grant: &Grant) -> Result<String> {
    if grant.objects.is_empty() {
        return Err(unsupported("a grant names at least one folder or project"));
    }
    if grant.access == Access::Unknown {
        return Err(unsupported("a grant is read-write (rw) or read-only (ro)"));
    }
    serde_json::to_string(grant).map_err(|e| unsupported(format!("grant: {e}")))
}

/// A session's end, counted from its last use. In hours, because an instant
/// has no calendar to count days in.
fn expiry(now: Timestamp) -> Result<Timestamp> {
    now.checked_add((SESSION_DAYS * 24).hours())
        .map_err(|e| unsupported(format!("session expiry: {e}")))
}

/// A new secret token, 32 random bytes in hex, as sessions get them; for
/// the local sessions of the replica on a computer (ADR 0067 §3).
pub fn new_token() -> Result<String> {
    Ok(hex(&random_bytes::<32>()?))
}

/// The SHA-256 of a token in hex: what is kept of it (ADR 0067 §3).
#[must_use]
pub fn token_digest(token: &str) -> String {
    hex(&token_hash(token))
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Whether `name` can be an account: it is also a file name.
#[must_use]
pub fn valid_account_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn hash_password(password: &str) -> Result<String> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(unsupported(format!(
            "a password needs at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    let salt = SaltString::encode_b64(&random_bytes::<16>()?)
        .map_err(|e| unsupported(format!("salt: {e}")))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| unsupported(format!("hashing the password: {e}")))?
        .to_string())
}

/// Whether `password` matches `stored`; `None` is checked against a fixed
/// hash and then refused, so the time an answer takes does not say which
/// accounts exist.
fn verify_password(stored: Option<&str>, password: &str) -> Result<bool> {
    let dummy = DUMMY_HASH.get_or_init(|| hash_password("samu: no such account").ok());
    let (hash, real) = match (stored, dummy) {
        (Some(stored), _) => (stored, true),
        (None, Some(dummy)) => (dummy.as_str(), false),
        (None, None) => return Ok(false),
    };
    let parsed =
        PasswordHash::new(hash).map_err(|_| unsupported("a stored password hash is unreadable"))?;
    let matches = Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok();
    Ok(real && matches)
}

impl ServerStore {
    /// Opens (or creates) the server file and migrates it.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = open::open(path)?;
        let migration = migrations::migrate_server(&conn)?;
        Ok(Self { conn, migration })
    }

    /// In memory, for tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = open::open_in_memory()?;
        let migration = migrations::migrate_server(&conn)?;
        Ok(Self { conn, migration })
    }

    #[must_use]
    pub fn migration(&self) -> &MigrationReport {
        &self.migration
    }

    /// Creates an account. Its database is the caller's to create.
    pub fn add_account(&mut self, name: &str, password: &str, now: Timestamp) -> Result<()> {
        self.add_account_as(name, password, Registered::Cli, now)
    }

    /// Creates an account that came to be as `registered` (ADR 0099 §4).
    pub fn add_account_as(
        &mut self,
        name: &str,
        password: &str,
        registered: Registered,
        now: Timestamp,
    ) -> Result<()> {
        if !valid_account_name(name) {
            return Err(unsupported(
                "an account name is 1 to 32 of a-z, 0-9, '_' and '-'",
            ));
        }
        let hash = hash_password(password)?;
        let inserted = self.conn.execute(
            "INSERT INTO account (name, password_hash, created, registered) \
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT (name) DO NOTHING",
            params![name, hash, now.to_string(), registered.as_str()],
        )?;
        if inserted == 0 {
            return Err(unsupported(format!("there is already an account {name:?}")));
        }
        Ok(())
    }

    /// Sets a new password and ends every session of the account.
    pub fn set_password(&mut self, name: &str, password: &str) -> Result<()> {
        let hash = hash_password(password)?;
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE account SET password_hash = ?2 WHERE name = ?1",
            params![name, hash],
        )?;
        if changed == 0 {
            return Err(unsupported(format!("there is no account {name:?}")));
        }
        tx.execute(
            "DELETE FROM session WHERE account = (SELECT id FROM account WHERE name = ?1)",
            [name],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Disables an account and ends its sessions.
    pub fn disable_account(&mut self, name: &str, now: Timestamp) -> Result<()> {
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE account SET disabled_at = coalesce(disabled_at, ?2) WHERE name = ?1",
            params![name, now.to_string()],
        )?;
        if changed == 0 {
            return Err(unsupported(format!("there is no account {name:?}")));
        }
        tx.execute(
            "DELETE FROM session WHERE account = (SELECT id FROM account WHERE name = ?1)",
            [name],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes an account's row: its sessions, hooks and pairing code go
    /// with it (ADR 0099 §5). Its files are the caller's, with
    /// [`crate::AccountStore::remove`], under the account's lock.
    pub fn delete_account(&mut self, name: &str) -> Result<()> {
        let deleted = self
            .conn
            .execute("DELETE FROM account WHERE name = ?1", [name])?;
        if deleted == 0 {
            return Err(unsupported(format!("there is no account {name:?}")));
        }
        Ok(())
    }

    /// One account, if there is one by that name.
    pub fn account_info(&self, name: &str) -> Result<Option<AccountInfo>> {
        Ok(self.accounts()?.into_iter().find(|a| a.name == name))
    }

    /// How many accounts came by `POST /register` (ADR 0099 §3); those from
    /// `samu-server account add` do not count.
    pub fn registered_count(&self) -> Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM account WHERE registered != 'cli'",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// Moves `last_active` of an account on to `now` (ADR 0099 §4).
    pub fn touch(&mut self, name: &str, now: Timestamp) -> Result<()> {
        self.conn.execute(
            "UPDATE account SET last_active = ?2 WHERE name = ?1",
            params![name, now.to_string()],
        )?;
        Ok(())
    }

    /// Whether `password` is the one of the active account `name`; an
    /// unknown name takes as long as a wrong password.
    pub fn check_password(&self, name: &str, password: &str) -> Result<bool> {
        let stored: Option<String> = self
            .conn
            .query_row(
                "SELECT password_hash FROM account WHERE name = ?1 AND disabled_at IS NULL",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        verify_password(stored.as_deref(), password)
    }

    /// A person's session for an account the caller has just made with its
    /// password, without checking that once more (ADR 0099 §2); `None` if
    /// there is no such active account.
    pub fn open_first_session(
        &mut self,
        name: &str,
        device: &str,
        now: Timestamp,
    ) -> Result<Option<(String, SessionRow)>> {
        let account: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM account WHERE name = ?1 AND disabled_at IS NULL",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        let Some(account) = account else {
            return Ok(None);
        };
        self.touch(name, now)?;
        Ok(Some(self.open_session(
            account,
            name,
            device,
            AuthorKind::Human,
            None,
            now,
        )?))
    }

    /// Every account, by name.
    pub fn accounts(&self) -> Result<Vec<AccountInfo>> {
        let mut statement = self.conn.prepare(
            "SELECT name, created, disabled_at IS NOT NULL, registered, \
             coalesce(last_active, created) FROM account ORDER BY name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let corrupt = |what: &str, e: &dyn std::fmt::Display| Error::Corrupt {
            what: format!("account.{what}: {e}"),
        };
        let mut accounts = Vec::new();
        for row in rows {
            let (name, created, disabled, registered, last_active) = row?;
            accounts.push(AccountInfo {
                created: created.parse().map_err(|e| corrupt("created", &e))?,
                last_active: last_active
                    .parse()
                    .map_err(|e| corrupt("last_active", &e))?,
                registered: Registered::parse(&registered)
                    .ok_or_else(|| corrupt("registered", &registered))?,
                name,
                disabled,
            });
        }
        Ok(accounts)
    }

    /// Whether any account may sign in; without one the server is closed
    /// (ADR 0009 §3).
    pub fn has_active_account(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM account WHERE disabled_at IS NULL)",
            [],
            |row| row.get(0),
        )?)
    }

    /// Signs in: the token of a new session, or `None` for a wrong name or
    /// password alike. With a grant the session is restricted (ADR 0095 §1);
    /// whether its objects belong to the account is the caller's to check.
    pub fn sign_in(
        &mut self,
        name: &str,
        password: &str,
        device: &str,
        kind: AuthorKind,
        grant: Option<&Grant>,
        now: Timestamp,
    ) -> Result<Option<(String, SessionRow)>> {
        let stored: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, password_hash FROM account WHERE name = ?1 AND disabled_at IS NULL",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if !verify_password(stored.as_ref().map(|(_, h)| h.as_str()), password)? {
            return Ok(None);
        }
        let Some((account, _)) = stored else {
            return Ok(None);
        };
        self.conn.execute(
            "UPDATE account SET last_active = ?2 WHERE id = ?1",
            params![account, now.to_string()],
        )?;
        Ok(Some(
            self.open_session(account, name, device, kind, grant, now)?,
        ))
    }

    /// A new session for an account already checked.
    fn open_session(
        &mut self,
        account: i64,
        name: &str,
        device: &str,
        kind: AuthorKind,
        grant: Option<&Grant>,
        now: Timestamp,
    ) -> Result<(String, SessionRow)> {
        let Some(kind_name) = author_kind_name(kind) else {
            return Err(unsupported("a session is a person's or an agent's"));
        };
        let grant_text = grant.map(grant_json).transpose()?;
        let token = hex(&random_bytes::<TOKEN_BYTES>()?);
        let expires = expiry(now)?;
        self.conn.execute(
            "INSERT INTO session (token_hash, account, device, kind, created, expires, \"grant\") \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                token_hash(&token),
                account,
                device,
                kind_name,
                now.to_string(),
                expires.to_string(),
                grant_text
            ],
        )?;
        Ok((
            token,
            SessionRow {
                account: name.into(),
                device: device.into(),
                kind,
                expires,
                grant: grant.cloned(),
            },
        ))
    }

    /// A pairing code for a signed-in account, after its password once more
    /// (ADR 0031 §1, §2); `None` for a wrong password or a disabled account.
    /// It replaces the account's open one.
    pub fn pairing_code(
        &mut self,
        name: &str,
        password: &str,
        device: &str,
        now: Timestamp,
    ) -> Result<Option<PairingCode>> {
        let device = device.trim();
        if device.is_empty() || device.chars().count() > 64 {
            return Err(unsupported("a device name has 1 to 64 characters"));
        }
        let stored: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, password_hash FROM account WHERE name = ?1 AND disabled_at IS NULL",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if !verify_password(stored.as_ref().map(|(_, h)| h.as_str()), password)? {
            return Ok(None);
        }
        let Some((account, _)) = stored else {
            return Ok(None);
        };
        let code = hex(&random_bytes::<PAIRING_BYTES>()?);
        let expires = now
            .checked_add(PAIRING_MINUTES.minutes())
            .map_err(|e| unsupported(format!("pairing expiry: {e}")))?;
        self.conn.execute(
            "INSERT INTO pairing_code (account, code_hash, device, created, expires) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(account) DO UPDATE SET code_hash = excluded.code_hash, \
             device = excluded.device, created = excluded.created, expires = excluded.expires",
            params![
                account,
                token_hash(&code),
                device,
                now.to_string(),
                expires.to_string()
            ],
        )?;
        Ok(Some(PairingCode {
            code,
            account: name.into(),
            device: device.into(),
            expires,
        }))
    }

    /// Trades a pairing code for a session, once (ADR 0031 §3); `None` for
    /// an unknown, expired or used code, or a disabled account, alike.
    pub fn redeem(&mut self, code: &str, now: Timestamp) -> Result<Option<(String, SessionRow)>> {
        // Expired ones go first. Compared as instants, not as text: the
        // fraction of a second has no fixed width.
        let open: Vec<(i64, String)> = {
            let mut statement = self
                .conn
                .prepare("SELECT account, expires FROM pairing_code")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (account, expires) in open {
            if expires.parse::<Timestamp>().map_or(true, |e| e <= now) {
                self.conn
                    .execute("DELETE FROM pairing_code WHERE account = ?1", [account])?;
            }
        }
        let found: Option<(i64, String, String)> = self
            .conn
            .query_row(
                "SELECT a.id, a.name, p.device FROM pairing_code p \
                 JOIN account a ON a.id = p.account \
                 WHERE p.code_hash = ?1 AND a.disabled_at IS NULL",
                [token_hash(code)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((account, name, device)) = found else {
            return Ok(None);
        };
        // Once: gone before the session is there.
        self.conn
            .execute("DELETE FROM pairing_code WHERE account = ?1", [account])?;
        // A paired phone is a person's (ADR 0039 §2).
        Ok(Some(self.open_session(
            account,
            &name,
            &device,
            AuthorKind::Human,
            // A pairing always makes a session of the whole account
            // (ADR 0095 §1).
            None,
            now,
        )?))
    }

    /// The live session behind a token, its expiry moved forward (sliding,
    /// ADR 0019 §3); `None` for an unknown, expired or disabled one.
    pub fn session(&mut self, token: &str, now: Timestamp) -> Result<Option<SessionRow>> {
        let hash = token_hash(token);
        let found: Option<(String, String, String, String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT a.name, s.device, s.expires, s.kind, s.\"grant\" FROM session s \
                 JOIN account a ON a.id = s.account \
                 WHERE s.token_hash = ?1 AND a.disabled_at IS NULL",
                [&hash],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((account, device, expires, kind, grant)) = found else {
            return Ok(None);
        };
        let expires: Timestamp = expires.parse().map_err(|e| Error::Corrupt {
            what: format!("session.expires: {e}"),
        })?;
        if expires <= now {
            self.conn
                .execute("DELETE FROM session WHERE token_hash = ?1", [&hash])?;
            return Ok(None);
        }
        let renewed = expiry(now)?;
        self.conn.execute(
            "UPDATE session SET expires = ?2 WHERE token_hash = ?1",
            params![hash, renewed.to_string()],
        )?;
        Ok(Some(SessionRow {
            account,
            device,
            kind: author_kind_of(&kind),
            expires: renewed,
            grant: grant
                .map(|text| {
                    serde_json::from_str(&text).map_err(|e| Error::Corrupt {
                        what: format!("session.grant: {e}"),
                    })
                })
                .transpose()?,
        }))
    }

    /// Ends a session. Whether there was one.
    pub fn sign_out(&mut self, token: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM session WHERE token_hash = ?1",
            [token_hash(token)],
        )? > 0)
    }

    /// The live sessions of `account`, oldest first (ADR 0095 §7); the one
    /// behind `current` is marked.
    pub fn sessions(
        &self,
        account: &str,
        current: &str,
        now: Timestamp,
    ) -> Result<Vec<SessionSummary>> {
        let current = token_hash(current);
        let mut statement = self.conn.prepare(
            "SELECT s.token_hash, s.device, s.kind, s.created, s.expires, s.\"grant\" \
             FROM session s JOIN account a ON a.id = s.account \
             WHERE a.name = ?1 ORDER BY s.created, s.token_hash",
        )?;
        let rows = statement.query_map([account], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            let (hash, device, kind, created, expires, grant) = row?;
            let corrupt = |what: &str, e: &dyn std::fmt::Display| Error::Corrupt {
                what: format!("session.{what}: {e}"),
            };
            let expires: Timestamp = expires.parse().map_err(|e| corrupt("expires", &e))?;
            if expires <= now {
                continue;
            }
            sessions.push(SessionSummary {
                id: hex(&hash)[..SESSION_ID_LEN].to_string(),
                device,
                kind: author_kind_of(&kind),
                created: created.parse().map_err(|e| corrupt("created", &e))?,
                expires,
                grant: grant
                    .map(|text| serde_json::from_str(&text).map_err(|e| corrupt("grant", &e)))
                    .transpose()?,
                current: hash == current,
            });
        }
        Ok(sessions)
    }

    /// Ends the session of `account` whose hash starts with `id`, in hex
    /// (ADR 0095 §7).
    pub fn end_session(&mut self, account: &str, id: &str) -> Result<Ended> {
        let id = id.trim().to_ascii_lowercase();
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(Ended::Unknown);
        }
        let hashes: Vec<Vec<u8>> = {
            let mut statement = self.conn.prepare(
                "SELECT s.token_hash FROM session s JOIN account a ON a.id = s.account \
                 WHERE a.name = ?1",
            )?;
            let rows = statement.query_map([account], |row| row.get::<_, Vec<u8>>(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let matching: Vec<&Vec<u8>> = hashes
            .iter()
            .filter(|hash| hex(hash).starts_with(&id))
            .collect();
        match matching.as_slice() {
            [] => Ok(Ended::Unknown),
            [hash] => {
                self.conn
                    .execute("DELETE FROM session WHERE token_hash = ?1", [hash])?;
                Ok(Ended::Ended)
            }
            _ => Ok(Ended::Ambiguous),
        }
    }

    /// Makes a hook for `account` and hands out its secret, this once.
    pub fn add_hook(
        &mut self,
        account: &str,
        name: &str,
        format: HookFormat,
        now: Timestamp,
    ) -> Result<(HookRow, String)> {
        validate_hook_name(name).map_err(|e| unsupported(e.to_string()))?;
        let format_text = format_name(format)?;
        let secret = hex(&random_bytes::<TOKEN_BYTES>()?);
        let inserted = self.conn.execute(
            "INSERT INTO hook (name, account, format, secret, created) \
             SELECT ?1, id, ?3, ?4, ?5 FROM account WHERE name = ?2 \
             ON CONFLICT (name) DO NOTHING",
            params![name, account, format_text, secret, now.to_string()],
        )?;
        if inserted == 0 {
            return Err(unsupported(format!(
                "there is already a hook {name:?}, or no account {account:?}"
            )));
        }
        let row = hook_row(name.into(), account.into(), format_text, &now.to_string())?;
        Ok((row, secret))
    }

    /// The devices of `account` that sign in as an agent (ADR 0039 §2): a
    /// field stamp from one of them is an agent's work (ADR 0080 §2).
    pub fn agent_devices(&self, account: &str) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
            "SELECT DISTINCT s.device FROM session s JOIN account a ON a.id = s.account \
             WHERE a.name = ?1 AND s.kind = 'agent' ORDER BY s.device",
        )?;
        let rows = statement.query_map([account], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The hooks of `account`, by name.
    pub fn hooks(&self, account: &str) -> Result<Vec<HookRow>> {
        let mut statement = self.conn.prepare(
            "SELECT h.name, h.format, h.created FROM hook h \
             JOIN account a ON a.id = h.account WHERE a.name = ?1 ORDER BY h.name",
        )?;
        let rows = statement.query_map([account], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut hooks = Vec::new();
        for row in rows {
            let (name, format, created) = row?;
            hooks.push(hook_row(name, account.into(), &format, &created)?);
        }
        Ok(hooks)
    }

    /// Removes a hook of `account`. Whether there was one.
    pub fn remove_hook(&mut self, account: &str, name: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM hook WHERE name = ?2 \
             AND account = (SELECT id FROM account WHERE name = ?1)",
            params![account, name],
        )? > 0)
    }

    /// Gives a hook of `account` a new secret; the old one stops working at
    /// once. `None` if `account` has no such hook.
    pub fn renew_hook_secret(
        &mut self,
        account: &str,
        name: &str,
    ) -> Result<Option<(HookRow, String)>> {
        let secret = hex(&random_bytes::<TOKEN_BYTES>()?);
        let changed = self.conn.execute(
            "UPDATE hook SET secret = ?3 WHERE name = ?2 \
             AND account = (SELECT id FROM account WHERE name = ?1)",
            params![account, name, secret],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(self.hook(name)?.map(|(row, _)| (row, secret)))
    }

    /// A hook by name with its secret, for a delivery; `None` for an
    /// unknown hook or one whose account is disabled.
    pub fn hook(&self, name: &str) -> Result<Option<(HookRow, String)>> {
        let found: Option<(String, String, String, String)> = self
            .conn
            .query_row(
                "SELECT a.name, h.format, h.secret, h.created FROM hook h \
                 JOIN account a ON a.id = h.account \
                 WHERE h.name = ?1 AND a.disabled_at IS NULL",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((account, format, secret, created)) = found else {
            return Ok(None);
        };
        Ok(Some((
            hook_row(name.into(), account, &format, &created)?,
            secret,
        )))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    const NOW: &str = "2026-11-07T09:00:00Z";
    const PASSWORD: &str = "correct horse";

    fn store_with_account() -> ServerStore {
        let mut store = ServerStore::open_in_memory().unwrap();
        store.add_account("privat", PASSWORD, at(NOW)).unwrap();
        store
    }

    /// ADR 0099 §4: an account from the CLI is `cli`; it is last active
    /// when it was made, then when it last signed in.
    #[test]
    fn an_account_knows_how_it_came_and_when_it_was_last_used() {
        let mut store = store_with_account();
        let listed = &store.accounts().unwrap()[0];
        assert_eq!(listed.registered, Registered::Cli);
        assert_eq!(listed.last_active, at(NOW));
        let later = "2026-11-09T18:30:00Z";
        assert!(
            store
                .sign_in(
                    "privat",
                    "wrong password",
                    "x",
                    AuthorKind::Human,
                    None,
                    at(later)
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(store.accounts().unwrap()[0].last_active, at(NOW));
        store
            .sign_in("privat", PASSWORD, "x", AuthorKind::Human, None, at(later))
            .unwrap()
            .unwrap();
        assert_eq!(store.accounts().unwrap()[0].last_active, at(later));
    }

    /// ADR 0099 §3–4: registered accounts are counted apart from those of
    /// the CLI; one is found by name, touched, its password checked, and a
    /// first session opened without the password.
    #[test]
    fn registered_accounts_are_counted_touched_and_signed_in() {
        let mut store = store_with_account();
        assert_eq!(store.registered_count().unwrap(), 0);
        store
            .add_account_as("demo-abc123", PASSWORD, Registered::Demo, at(NOW))
            .unwrap();
        store
            .add_account_as("verein", PASSWORD, Registered::Open, at(NOW))
            .unwrap();
        assert_eq!(store.registered_count().unwrap(), 2);
        let demo = store.account_info("demo-abc123").unwrap().unwrap();
        assert_eq!(demo.registered, Registered::Demo);
        assert!(store.account_info("nobody").unwrap().is_none());

        let later = "2026-11-08T10:00:00Z";
        store.touch("demo-abc123", at(later)).unwrap();
        let demo = store.account_info("demo-abc123").unwrap().unwrap();
        assert_eq!(demo.last_active, at(later));

        assert!(store.check_password("verein", PASSWORD).unwrap());
        assert!(!store.check_password("verein", "wrong password").unwrap());
        assert!(!store.check_password("nobody", PASSWORD).unwrap());

        let (token, row) = store
            .open_first_session("verein", "web", at(later))
            .unwrap()
            .unwrap();
        assert_eq!(row.kind, AuthorKind::Human);
        assert!(row.grant.is_none());
        assert!(store.session(&token, at(later)).unwrap().is_some());
        assert!(
            store
                .open_first_session("nobody", "web", at(later))
                .unwrap()
                .is_none()
        );
    }

    /// ADR 0099 §5: deleting an account ends its sessions; the name is free
    /// again; an unknown name is an error.
    #[test]
    fn deleting_an_account_ends_its_sessions_and_frees_the_name() {
        let mut store = store_with_account();
        let (token, _) = store
            .sign_in("privat", PASSWORD, "x", AuthorKind::Human, None, at(NOW))
            .unwrap()
            .unwrap();
        store.delete_account("privat").unwrap();
        assert!(store.accounts().unwrap().is_empty());
        assert!(store.session(&token, at(NOW)).unwrap().is_none());
        let sessions: i64 = store
            .conn
            .query_row("SELECT count(*) FROM session", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sessions, 0);
        assert!(store.delete_account("privat").is_err());
        store.add_account("privat", PASSWORD, at(NOW)).unwrap();
    }

    /// ADR 0039 §2: a session keeps the kind it signed in with; a paired
    /// phone is a person's; a kind the server does not know signs nobody in.
    #[test]
    fn a_session_keeps_its_kind() {
        let mut store = store_with_account();
        let (agent, row) = store
            .sign_in(
                "privat",
                PASSWORD,
                "claude",
                AuthorKind::Agent,
                None,
                at(NOW),
            )
            .unwrap()
            .unwrap();
        assert_eq!(row.kind, AuthorKind::Agent);
        assert_eq!(
            store.session(&agent, at(NOW)).unwrap().unwrap().kind,
            AuthorKind::Agent
        );
        let (person, _) = store
            .sign_in(
                "privat",
                PASSWORD,
                "laptop",
                AuthorKind::Human,
                None,
                at(NOW),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            store.session(&person, at(NOW)).unwrap().unwrap().kind,
            AuthorKind::Human
        );
        let code = store
            .pairing_code("privat", PASSWORD, "pixel", at(NOW))
            .unwrap()
            .unwrap();
        let (_, paired) = store.redeem(&code.code, at(NOW)).unwrap().unwrap();
        assert_eq!(paired.kind, AuthorKind::Human);
        assert!(
            store
                .sign_in("privat", PASSWORD, "x", AuthorKind::Unknown, None, at(NOW))
                .is_err()
        );
        // ADR 0080 §2: the agent's device, and no one else's.
        assert_eq!(store.agent_devices("privat").unwrap(), vec!["claude"]);
        assert!(store.agent_devices("nobody").unwrap().is_empty());
    }

    /// ADR 0039 §2: a session from before the kind is a person's.
    #[test]
    fn a_session_from_before_is_a_persons() {
        let conn = Connection::open_in_memory().unwrap();
        for sql in [
            include_str!("migrations/server_0001.sql"),
            include_str!("migrations/server_0002.sql"),
            include_str!("migrations/server_0003.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(
            "PRAGMA user_version = 3; \
             INSERT INTO account (id, name, password_hash, created) \
             VALUES (1, 'privat', 'x', '2026-11-07T09:00:00Z'); \
             INSERT INTO session VALUES (zeroblob(32), 1, 'pixel', \
             '2026-11-07T09:00:00Z', '2026-12-07T09:00:00Z');",
        )
        .unwrap();
        let report = migrations::migrate_server(&conn).unwrap();
        assert_eq!(
            report.applied,
            vec![
                "0004_session_kind",
                "0005_session_grant",
                "0006_account_registered"
            ]
        );
        let kind: String = conn
            .query_row("SELECT kind FROM session", [], |row| row.get(0))
            .unwrap();
        assert_eq!(kind, "human");
    }

    fn grant(access: Access) -> Grant {
        Grant {
            objects: vec![
                "01a1bb73-e364-7cd8-b9e3-000000000001".parse().unwrap(),
                "01a1bb73-e364-7cd8-b9e3-000000000002".parse().unwrap(),
            ],
            access,
        }
    }

    /// ADR 0095 §1: a grant is kept with the session and comes back with
    /// it; without one the session is the whole account's, and so is every
    /// session from a pairing.
    #[test]
    fn a_session_keeps_its_grant() {
        let mut store = store_with_account();
        for access in [Access::Rw, Access::Ro] {
            let given = grant(access);
            let (token, row) = store
                .sign_in(
                    "privat",
                    PASSWORD,
                    "claude",
                    AuthorKind::Agent,
                    Some(&given),
                    at(NOW),
                )
                .unwrap()
                .unwrap();
            assert_eq!(row.grant.as_ref(), Some(&given));
            let again = store.session(&token, at(NOW)).unwrap().unwrap();
            assert_eq!(again.grant, Some(given));
        }
        let (whole, row) = store
            .sign_in(
                "privat",
                PASSWORD,
                "laptop",
                AuthorKind::Human,
                None,
                at(NOW),
            )
            .unwrap()
            .unwrap();
        assert_eq!(row.grant, None);
        assert_eq!(store.session(&whole, at(NOW)).unwrap().unwrap().grant, None);
        let code = store
            .pairing_code("privat", PASSWORD, "pixel", at(NOW))
            .unwrap()
            .unwrap();
        let (paired, _) = store.redeem(&code.code, at(NOW)).unwrap().unwrap();
        assert_eq!(
            store.session(&paired, at(NOW)).unwrap().unwrap().grant,
            None
        );
    }

    /// ADR 0095 §1: a grant names something and an access this server
    /// knows; otherwise no session is made.
    #[test]
    fn an_empty_or_unknown_grant_signs_nobody_in() {
        let mut store = store_with_account();
        let empty = Grant {
            objects: Vec::new(),
            access: Access::Rw,
        };
        for bad in [empty, grant(Access::Unknown)] {
            assert!(
                store
                    .sign_in(
                        "privat",
                        PASSWORD,
                        "claude",
                        AuthorKind::Agent,
                        Some(&bad),
                        at(NOW)
                    )
                    .is_err()
            );
        }
        let count: i64 = store
            .conn
            .query_row("SELECT count(*) FROM session", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    /// ADR 0031 §2, §3: the password once more, then once and within ten
    /// minutes; a new code replaces the open one.
    #[test]
    fn a_pairing_code_needs_the_password_and_pairs_once() {
        let mut store = store_with_account();
        assert!(
            store
                .pairing_code("privat", "wrong password", "pixel", at(NOW))
                .unwrap()
                .is_none()
        );
        let first = store
            .pairing_code("privat", PASSWORD, "pixel", at(NOW))
            .unwrap()
            .unwrap();
        assert_eq!(first.code.len(), 32);
        assert_eq!(first.expires, at("2026-11-07T09:10:00Z"));
        let second = store
            .pairing_code("privat", PASSWORD, " tablet ", at(NOW))
            .unwrap()
            .unwrap();
        assert_ne!(first.code, second.code);
        assert!(
            store.redeem(&first.code, at(NOW)).unwrap().is_none(),
            "the new one replaced it"
        );

        let (token, session) = store
            .redeem(&second.code, at("2026-11-07T09:05:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!(
            (session.account.as_str(), session.device.as_str()),
            ("privat", "tablet")
        );
        assert!(store.session(&token, at(NOW)).unwrap().is_some());
        assert!(
            store
                .redeem(&second.code, at("2026-11-07T09:06:00Z"))
                .unwrap()
                .is_none(),
            "once"
        );

        // Asked of the database: no code is left, no code in the clear.
        let rows: i64 = store
            .conn
            .query_row("SELECT count(*) FROM pairing_code", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn a_pairing_code_expires_and_a_disabled_account_cannot_pair() {
        let mut store = store_with_account();
        let code = store
            .pairing_code("privat", PASSWORD, "pixel", at(NOW))
            .unwrap()
            .unwrap();
        assert!(
            store
                .redeem(&code.code, at("2026-11-07T09:10:00Z"))
                .unwrap()
                .is_none()
        );
        let rows: i64 = store
            .conn
            .query_row("SELECT count(*) FROM pairing_code", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "the expired one is cleared");

        let code = store
            .pairing_code("privat", PASSWORD, "pixel", at(NOW))
            .unwrap()
            .unwrap();
        store.disable_account("privat", at(NOW)).unwrap();
        assert!(store.redeem(&code.code, at(NOW)).unwrap().is_none());
        assert!(
            store
                .pairing_code("privat", PASSWORD, "pixel", at(NOW))
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .pairing_code("privat", PASSWORD, "  ", at(NOW))
                .is_err()
        );
    }

    #[test]
    fn no_account_means_closed() {
        let store = ServerStore::open_in_memory().unwrap();
        assert!(!store.has_active_account().unwrap());
        assert!(store_with_account().has_active_account().unwrap());
    }

    #[test]
    fn a_right_password_opens_a_session_and_a_wrong_one_does_not() {
        let mut store = store_with_account();
        let (token, session) = store
            .sign_in(
                "privat",
                PASSWORD,
                "laptop",
                AuthorKind::Human,
                None,
                at(NOW),
            )
            .unwrap()
            .unwrap();
        assert_eq!(session.account, "privat");
        assert_eq!(token.len(), 64);
        assert!(
            store
                .sign_in(
                    "privat",
                    "wrong password",
                    "laptop",
                    AuthorKind::Human,
                    None,
                    at(NOW)
                )
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .sign_in(
                    "nobody",
                    PASSWORD,
                    "laptop",
                    AuthorKind::Human,
                    None,
                    at(NOW)
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store.session(&token, at(NOW)).unwrap().unwrap().device,
            "laptop"
        );
        assert!(store.session("not a token", at(NOW)).unwrap().is_none());
    }

    #[test]
    fn a_session_slides_and_expires_unused() {
        let mut store = store_with_account();
        let (token, _) = store
            .sign_in(
                "privat",
                PASSWORD,
                "laptop",
                AuthorKind::Human,
                None,
                at(NOW),
            )
            .unwrap()
            .unwrap();
        // Used after 20 days: moved forward.
        let used = store
            .session(&token, at("2026-11-27T09:00:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!(used.expires, at("2026-12-27T09:00:00Z"));
        // Unused for more than 30 days after that: gone.
        assert!(
            store
                .session(&token, at("2027-01-30T09:00:00Z"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_new_password_or_disabling_ends_the_sessions() {
        let mut store = store_with_account();
        let (token, _) = store
            .sign_in(
                "privat",
                PASSWORD,
                "laptop",
                AuthorKind::Human,
                None,
                at(NOW),
            )
            .unwrap()
            .unwrap();
        store.set_password("privat", "another password").unwrap();
        assert!(store.session(&token, at(NOW)).unwrap().is_none());

        let (token, _) = store
            .sign_in(
                "privat",
                "another password",
                "laptop",
                AuthorKind::Human,
                None,
                at(NOW),
            )
            .unwrap()
            .unwrap();
        store.disable_account("privat", at(NOW)).unwrap();
        assert!(store.session(&token, at(NOW)).unwrap().is_none());
        assert!(!store.has_active_account().unwrap());
        assert!(
            store
                .sign_in(
                    "privat",
                    "another password",
                    "laptop",
                    AuthorKind::Human,
                    None,
                    at(NOW)
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn sign_out_ends_one_session() {
        let mut store = store_with_account();
        let (token, _) = store
            .sign_in(
                "privat",
                PASSWORD,
                "laptop",
                AuthorKind::Human,
                None,
                at(NOW),
            )
            .unwrap()
            .unwrap();
        assert!(store.sign_out(&token).unwrap());
        assert!(!store.sign_out(&token).unwrap());
        assert!(store.session(&token, at(NOW)).unwrap().is_none());
    }

    #[test]
    fn names_and_passwords_are_checked() {
        let mut store = ServerStore::open_in_memory().unwrap();
        for bad in ["", "Privat", "../x", "a b", &"x".repeat(33)] {
            assert!(
                store.add_account(bad, PASSWORD, at(NOW)).is_err(),
                "{bad:?}"
            );
        }
        assert!(store.add_account("privat", "short", at(NOW)).is_err());
        store.add_account("privat", PASSWORD, at(NOW)).unwrap();
        assert!(
            store.add_account("privat", PASSWORD, at(NOW)).is_err(),
            "twice"
        );
    }

    #[test]
    fn a_hook_belongs_to_one_account_and_hands_out_its_secret_once() {
        let mut store = store_with_account();
        store.add_account("dienst", PASSWORD, at(NOW)).unwrap();
        let (row, secret) = store
            .add_hook("privat", "ring", HookFormat::IndexV1, at(NOW))
            .unwrap();
        assert_eq!(row.account, "privat");
        assert_eq!(secret.len(), 64);
        let (found, stored) = store.hook("ring").unwrap().unwrap();
        assert_eq!((found, stored), (row.clone(), secret.clone()));
        assert_eq!(store.hooks("privat").unwrap(), vec![row]);
        assert!(store.hooks("dienst").unwrap().is_empty());
        // A name is unique on the server, whoever asks.
        assert!(
            store
                .add_hook("dienst", "ring", HookFormat::IndexV1, at(NOW))
                .is_err()
        );
        // Another account cannot renew or remove it.
        assert_eq!(store.renew_hook_secret("dienst", "ring").unwrap(), None);
        assert!(!store.remove_hook("dienst", "ring").unwrap());
        let (_, renewed) = store.renew_hook_secret("privat", "ring").unwrap().unwrap();
        assert_ne!(renewed, secret);
        assert_eq!(store.hook("ring").unwrap().unwrap().1, renewed);
        assert!(store.remove_hook("privat", "ring").unwrap());
        assert_eq!(store.hook("ring").unwrap(), None);
    }

    #[test]
    fn a_hook_of_a_disabled_account_takes_nothing() {
        let mut store = store_with_account();
        store
            .add_hook("privat", "ring", HookFormat::IndexV1, at(NOW))
            .unwrap();
        assert!(store.hook("ring").unwrap().is_some());
        store.disable_account("privat", at(NOW)).unwrap();
        assert_eq!(store.hook("ring").unwrap(), None);
    }

    #[test]
    fn hook_names_and_formats_are_checked() {
        let mut store = store_with_account();
        for bad in ["Ring", "", "a/b"] {
            assert!(
                store
                    .add_hook("privat", bad, HookFormat::IndexV1, at(NOW))
                    .is_err(),
                "{bad:?}"
            );
        }
        assert!(
            store
                .add_hook("privat", "ring", HookFormat::Unknown, at(NOW))
                .is_err()
        );
        assert!(
            store
                .add_hook("nobody", "ring", HookFormat::IndexV1, at(NOW))
                .is_err()
        );
        assert!(store.hooks("privat").unwrap().is_empty());
    }

    #[test]
    fn only_the_hash_of_a_token_is_stored() {
        let mut store = store_with_account();
        let (token, _) = store
            .sign_in(
                "privat",
                PASSWORD,
                "laptop",
                AuthorKind::Human,
                None,
                at(NOW),
            )
            .unwrap()
            .unwrap();
        let stored: Vec<u8> = store
            .conn
            .query_row("SELECT token_hash FROM session", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, token_hash(&token));
        assert_ne!(stored, token.as_bytes());
    }

    /// ADR 0095 §7: the sessions of an account by a prefix of their hash,
    /// without a token; another account's are not among them, a prefix
    /// that fits two ends neither, and an expired one is not listed.
    #[test]
    fn sessions_are_listed_and_ended_by_prefix() {
        let mut store = store_with_account();
        store.add_account("dienst", PASSWORD, at(NOW)).unwrap();
        let grant = Grant {
            objects: vec![crate::core::api::Id::new()],
            access: Access::Ro,
        };
        let sign = |store: &mut ServerStore, account: &str, device: &str, grant: Option<&Grant>| {
            store
                .sign_in(account, PASSWORD, device, AuthorKind::Human, grant, at(NOW))
                .unwrap()
                .unwrap()
                .0
        };
        let laptop = sign(&mut store, "privat", "laptop", None);
        let claude = sign(&mut store, "privat", "claude", Some(&grant));
        let other = sign(&mut store, "dienst", "laptop", None);
        let listed = store.sessions("privat", &laptop, at(NOW)).unwrap();
        assert_eq!(listed.len(), 2);
        let mine = listed.iter().find(|s| s.current).unwrap();
        assert_eq!(mine.device, "laptop");
        assert_eq!(mine.id, token_digest(&laptop)[..SESSION_ID_LEN]);
        let restricted = listed.iter().find(|s| !s.current).unwrap();
        assert_eq!(restricted.grant.as_ref(), Some(&grant));
        let json = serde_json::to_string(&listed).unwrap();
        assert!(!json.contains(&laptop) && !json.contains(&claude));

        let later = at("2027-01-01T00:00:00Z");
        assert!(store.sessions("privat", &laptop, later).unwrap().is_empty());

        assert_eq!(store.end_session("privat", "").unwrap(), Ended::Unknown);
        assert_eq!(store.end_session("privat", "zz").unwrap(), Ended::Unknown);
        let foreign = &token_digest(&other)[..SESSION_ID_LEN];
        assert_eq!(
            store.end_session("privat", foreign).unwrap(),
            Ended::Unknown
        );
        // Seventeen sessions: two of them share their first hex digit.
        let many: Vec<String> = (0..17)
            .map(|n| sign(&mut store, "privat", &format!("d{n}"), None))
            .collect();
        let shared = "0123456789abcdef"
            .chars()
            .map(String::from)
            .find(|d| {
                many.iter()
                    .filter(|t| token_digest(t).starts_with(d))
                    .count()
                    > 1
            })
            .unwrap();
        assert_eq!(
            store.end_session("privat", &shared).unwrap(),
            Ended::Ambiguous
        );
        assert_eq!(
            store.sessions("privat", &laptop, at(NOW)).unwrap().len(),
            19
        );
        let id = token_digest(&claude)[..SESSION_ID_LEN].to_ascii_uppercase();
        assert_eq!(store.end_session("privat", &id).unwrap(), Ended::Ended);
        assert!(store.session(&claude, at(NOW)).unwrap().is_none());
        assert!(store.session(&laptop, at(NOW)).unwrap().is_some());
        assert!(store.session(&other, at(NOW)).unwrap().is_some());
    }
}
