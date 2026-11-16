//! The samu command line. It covers the whole API, one command per route
//! (ADR 0019 §7, catastr ADR 0009).
//!
//! - Data on stdout, messages on stderr.
//! - `--format plain|json|jsonl|tsv`.
//! - Exit codes are defined and tested, see [`exit`].

mod appointment;
mod blockers;
mod bulk;
mod calendar;
mod clock;
mod comments;
mod complete;
mod config;
mod export;
mod export_org;
mod forecast;
mod grant;
mod habits;
mod import;
mod org;
mod output;
mod perspectives;
mod places;
mod profiles;
mod props;
mod remind;
mod report;
mod ring;
mod scopes;
mod table;
mod tags;
mod templates;

use std::io::{BufRead, IsTerminal, Read};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;
use samu_api::{
    AnyObject, BareStatus, BlockSection, CaptureRequest, CaptureResponse, CaptureWarning, Change,
    ChangeOutcome, ChangesPage, DeleteRequest, DeleteResponse, Hlc, HookChange, HookChangeResponse,
    HookDelivery, HookFormat, HookView, Id, LogEntry, Login, ObjectKind, PairCode, PairCodeRequest,
    PairRequest, ProjectAvailability, RenderResponse, SessionInfo, TaskFate, TaskStatus,
};
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use samu_client::{Client, Error as Failure};

/// The exit codes (ADR 0019 §7).
pub mod exit {
    /// Everything worked.
    pub const SUCCESS: u8 = 0;
    /// Something went wrong that none of the codes below describes.
    pub const FAILURE: u8 = 1;
    /// Wrong invocation. Emitted by clap itself.
    pub const USAGE: u8 = 2;
    /// What was asked for does not exist.
    pub const NOT_FOUND: u8 = 3;
    /// The server could not be reached.
    pub const NO_SERVER: u8 = 4;
    // 5 was a second `samu import org`; it runs as often as wanted now
    // (ADR 0051), and 5 stays unused so old scripts do not misread it.
    /// Not signed in, or signing in failed.
    pub const UNAUTHORIZED: u8 = 6;
}

/// The samu command line.
#[derive(Debug, Parser)]
#[command(name = "samu", version, about)]
struct Args {
    /// The server, like https://samu.example.org.
    #[arg(long, global = true)]
    server: Option<String>,
    /// The account to act on.
    #[arg(long, global = true)]
    account: Option<String>,
    /// A profile of the configuration file: an account on this computer
    /// with its service on the loopback (ADR 0086).
    #[arg(long, global = true)]
    profile: Option<String>,
    /// How data is printed.
    #[arg(long, global = true, value_enum, default_value = "plain")]
    format: Format,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Whether the server answers (GET /health).
    Health,
    /// Sign in to an account (POST /login), or with a pairing code from
    /// `samu pair` or the web UI (POST /pair, ADR 0031).
    Login {
        /// Read the password from the first line of stdin.
        #[arg(long)]
        password_stdin: bool,
        /// Trade this pairing code for the session instead of a password;
        /// the device name is the code's.
        #[arg(long, conflicts_with_all = ["password_stdin", "device"])]
        code: Option<String>,
        /// The device name for this session; default from the config or the
        /// host name.
        #[arg(long)]
        device: Option<String>,
        /// Sign in as an agent, not a person (ADR 0039 §2): what it writes
        /// carries that as its author.
        #[arg(long, conflicts_with = "code")]
        agent: bool,
        /// Restrict the session to these folders and projects with all
        /// below them (ADR 0095): a path as `samu folder list` shows it
        /// (`tec/ai`), a project as `tec/ai/samudo` or by title, or an id.
        #[arg(long, num_args = 1.., conflicts_with = "code")]
        only: Vec<String>,
        /// With `--only`: read, never write.
        #[arg(long, requires = "only")]
        read_only: bool,
    },
    /// Sign out (POST /logout).
    Logout,
    /// Make an account on a server that allows it (POST /register,
    /// ADR 0099), signed in at once; `--info` says what it allows
    /// (GET /register). Without a name, in the mode `demo`: a demo account
    /// with a made name and password, printed this once.
    Register {
        /// The account's name: 1 to 32 of a-z, 0-9, '_' and '-'.
        name: Option<String>,
        /// Read the password from the first line of stdin.
        #[arg(long, requires = "name")]
        password_stdin: bool,
        /// Demo data in this language: de or en. A demo account always gets
        /// some.
        #[arg(long, value_parser = ["de", "en"])]
        demo: Option<String>,
        /// The device name for the session; default from the config or the
        /// host name.
        #[arg(long)]
        device: Option<String>,
        /// Only what the server allows: off, demo or open.
        #[arg(long, conflicts_with_all = ["name", "password_stdin", "demo", "device"])]
        info: bool,
    },
    /// The signed-in account itself.
    #[command(subcommand)]
    Account(AccountCommand),
    /// Demo data into the signed-in account (POST /demo, ADR 0099 §7):
    /// everything into an empty account, else only under a folder and a
    /// tag `Demo`, which go again with `folder delete`. Prints how many
    /// of each were made.
    Demo {
        /// The language of the demo data.
        #[arg(value_parser = ["de", "en"])]
        lang: String,
    },
    /// A pairing code for a phone, after the password once more
    /// (POST /pair/code, ADR 0031): ten minutes, once. Prints the URI, the
    /// code and, on a terminal, the QR code.
    Pair {
        /// The name the phone will have.
        device: String,
        /// Read the password from the first line of stdin.
        #[arg(long)]
        password_stdin: bool,
    },
    /// The current session (GET /session); `end` ends another one.
    Session {
        #[command(subcommand)]
        action: Option<SessionCommand>,
    },
    /// The live sessions of the account, without tokens (GET /sessions,
    /// ADR 0095 §7). Only from a person's session of the whole account.
    Sessions,
    /// The profiles of this computer (ADR 0086).
    #[command(subcommand)]
    Profile(profiles::ProfileCommand),
    /// Add a task by quick capture (POST /capture, ADR 0011, ADR 0027):
    /// `#project @tag ! !A d:fr s:mo p:morgen ~15m every:1w // note`.
    Add {
        /// The text; several words are joined.
        #[arg(required = true, add = ArgValueCompleter::new(complete::capture))]
        title: Vec<String>,
        /// The project when the text names none, by title, id or the end of
        /// one.
        #[arg(long, add = ArgValueCompleter::new(complete::project))]
        project: Option<String>,
        /// An open task to put the new one under, as its last child and in
        /// its project (ADR 0062 §3); a project in the text wins.
        #[arg(long, conflicts_with = "literal", add = ArgValueCompleter::new(complete::task))]
        parent: Option<String>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        flag: bool,
        /// Show what would be made; write nothing.
        #[arg(long, conflicts_with = "literal")]
        dry_run: bool,
        /// Take the text as the title, word for word (POST /changes).
        #[arg(long)]
        literal: bool,
        /// Leave out what the scope chosen here gives a capture
        /// (ADR 0043 §6).
        #[arg(long)]
        no_scope: bool,
        /// A template by name (ADR 0073), the same as `/name` in the text.
        #[arg(long, conflicts_with = "literal")]
        template: Option<String>,
    },
    /// Edit a task by a line of quick capture with only marks (POST
    /// /capture with `task`, ADR 0078): `@tag` adds, `#project !A d:fr
    /// ~30m every:1w !` overwrite, `// text` is appended to the note,
    /// `/name` applies a template. The title is `samu set <id> title`.
    Edit {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        /// The marks; several words are joined.
        #[arg(required = true, add = ArgValueCompleter::new(complete::capture))]
        line: Vec<String>,
        /// Show the task as it would be; write nothing.
        #[arg(long)]
        preview: bool,
    },
    /// A capture template (ADR 0073): a line of quick capture with only
    /// marks, called as `/name` in `samu add`.
    #[command(subcommand)]
    Template(templates::TemplateCommand),
    /// A habit (ADR 0091): a task with a goal, entered day by day, with
    /// its streak computed on the server.
    #[command(subcommand)]
    Habit(habits::HabitCommand),
    /// A perspective (ADR 0090): a saved search with its filter,
    /// arrangement and presentation; `samu list --perspective` shows it.
    #[command(subcommand)]
    Perspective(perspectives::PerspectiveCommand),
    /// A note or comment as a document tree (POST /render, ADR 0058): the
    /// format it is read as, and with `--format json` the tree.
    Render {
        /// The file, `-` for stdin.
        #[arg(default_value = "-")]
        file: String,
        /// Read it as this instead of what it looks like.
        #[arg(long = "as", value_parser = ["org", "markdown"])]
        read_as: Option<String>,
    },
    /// Set one field of an object; the value is JSON, else a string. A
    /// property is the field `properties.<key>` (ADR 0023).
    Set {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        field: String,
        value: String,
    },
    /// Clear one field of an object, or remove a property
    /// (`properties.<key>`, ADR 0023).
    Unset {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        field: String,
    },
    /// Mark a task completed.
    Done {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        /// Why, for the logbook.
        #[arg(long)]
        note: Option<String>,
    },
    /// Mark a task dropped.
    Drop {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        /// Why, for the logbook.
        #[arg(long)]
        note: Option<String>,
    },
    /// Put a task into a state of its workflow, the next one, or none
    /// (ADR 0024).
    State {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        /// The state, by title or key; without it, `--next` or `--none`.
        #[arg(add = ArgValueCompleter::new(complete::state))]
        state: Option<String>,
        /// The next state, round to none (like `C-c C-t`).
        #[arg(long, conflicts_with_all = ["state", "none"])]
        next: bool,
        /// Out of every state.
        #[arg(long, conflicts_with = "state")]
        none: bool,
        /// Why, for the logbook.
        #[arg(long)]
        note: Option<String>,
    },
    /// Let a task wait until another is done: a `blocked_by` link
    /// (ADR 0017 §7). Refused for the task itself, twice, or in a circle.
    Block {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        /// The task it waits for.
        #[arg(add = ArgValueCompleter::new(complete::task))]
        blocker: String,
    },
    /// Take a blocker away again.
    Unblock {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        #[arg(add = ArgValueCompleter::new(complete::task))]
        blocker: String,
    },
    /// The tasks a task waits for, open or done.
    Blockers {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
    },
    /// What happened to an object, oldest first (GET /objects/{id}/logbook).
    Log {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
    },
    /// Tags and workflows (ADR 0024).
    #[command(subcommand)]
    Tag(tags::TagCommand),
    /// Set a reminder on a task or project (ADR 0030): at an instant, or
    /// before its due, planned or defer date. A due date reminds by itself.
    Remind(remind::Remind),
    /// The reminders that are set, of every task or of one.
    Reminders {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: Option<String>,
    },
    /// Write a comment on a task or project (ADR 0039 §3); `-` reads the
    /// text from stdin. Change it with `samu set <id> body`.
    Comment {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        text: String,
    },
    /// The comments on a task or project, oldest first.
    Comments {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
    },
    /// Attach a file to a task, a project or one of its comments
    /// (PUT /blobs/{sha256}, then the attachment; ADR 0039 §5).
    Attach(comments::Attach),
    /// The attachments of a task or project and of its comments.
    Attachments {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
    },
    /// The files behind attachments (ADR 0039 §4).
    #[command(subcommand)]
    Blob(comments::BlobCommand),
    /// Delete an object with what hangs on it (POST /objects/{id}/delete,
    /// ADR 0036): a project's tasks go to the inbox unless `--tasks` says
    /// otherwise, a folder's content moves up, a task takes its children,
    /// a tag comes off everything. It stays as a tombstone.
    Delete {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        /// Only for a project: `inbox`, `delete` (along with it), or another
        /// project by title or id.
        #[arg(long, add = ArgValueCompleter::new(complete::project))]
        tasks: Option<String>,
        /// Count what would happen; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Projects: make one, rename one (ADR 0036 §7).
    #[command(subcommand)]
    Project(places::ProjectCommand),
    /// Folders: make one, rename one, list them (ADR 0036 §7).
    #[command(subcommand)]
    Folder(places::FolderCommand),
    /// The properties of a task or project, its own and those it inherits
    /// from its action groups and project, with where each leads
    /// (ADR 0054).
    Prop {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
    },
    /// Open a property's value: this device's command for the key (`[open]`
    /// in the config file), else `xdg-open` on where it leads (ADR 0054 §5).
    Open {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
        key: String,
        /// Only say what would run.
        #[arg(long)]
        print: bool,
    },
    /// The handler for `samu-open:<uuid>/<key>`, the address the web hands a
    /// click to (ADR 0054 §6), which opens like `samu open`; and for
    /// `samu:<uuid>`, which opens the object in the web UI (ADR 0075 §2).
    OpenUri {
        uri: String,
        /// Only say what would run.
        #[arg(long)]
        print: bool,
        /// Also show a failure with notify-send (for the desktop handler).
        #[arg(long)]
        notify: bool,
    },
    /// Show one object (GET /objects/{id}).
    Show {
        #[arg(add = ArgValueCompleter::new(complete::task))]
        id: String,
    },
    /// What changed after a change number (GET /changes).
    Changes {
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
    /// Open tasks with their availability (GET /available).
    List {
        /// Only available tasks.
        #[arg(long)]
        available: bool,
        /// Only tasks planned for today or earlier.
        #[arg(long)]
        today: bool,
        /// Only tasks of a project, by title (or a unique start), id or the
        /// end of one.
        #[arg(long, add = ArgValueCompleter::new(complete::project))]
        project: Option<String>,
        /// Completed and dropped tasks too.
        #[arg(long)]
        all: bool,
        /// Only tasks with this tag or one below it, by title or path a/b.
        #[arg(long, add = ArgValueCompleter::new(complete::tag_path))]
        tag: Option<String>,
        /// Only flagged tasks.
        #[arg(long)]
        flagged: bool,
        /// Only tasks with every word in title, note or id.
        #[arg(long)]
        search: Option<String>,
        /// Through this scope instead of the one chosen here (ADR 0035).
        #[arg(long, conflicts_with = "no_scope")]
        scope: Option<String>,
        /// Without a scope, whatever is chosen here.
        #[arg(long)]
        no_scope: bool,
        /// What this perspective shows, by name or the end of its id, inside
        /// the scope (ADR 0090 §2).
        #[arg(long)]
        perspective: Option<String>,
        /// Only the id and what the server computed, not the task itself
        /// (GET /available?bare=true, ADR 0065).
        #[arg(long, conflicts_with_all = ["tag", "flagged", "search", "columns"])]
        bare: bool,
        /// As a table (ADR 0115 §8): the columns after the title, by the
        /// names of the web, like `state,due,estimate,worked,prop:jira`.
        /// Without a value the default ones; a table perspective brings
        /// its own.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        columns: Option<String>,
    },
    /// Projects with their availability (GET /available/projects).
    Projects {
        /// Only stuck projects: active, but without a next action.
        #[arg(long)]
        stuck: bool,
        /// Only projects due for review, oldest first (ADR 0088).
        #[arg(long)]
        review: bool,
        /// Completed and dropped projects too.
        #[arg(long)]
        all: bool,
        /// Through this scope instead of the one chosen here (ADR 0035).
        #[arg(long, conflicts_with = "no_scope")]
        scope: Option<String>,
        /// Without a scope, whatever is chosen here.
        #[arg(long)]
        no_scope: bool,
    },
    /// Mark projects reviewed (ADR 0088 §3): `last_reviewed` now and
    /// `next_review` cleared, so the next review counts from today.
    Review {
        /// Projects by id, `samu:` URI or title.
        #[arg(required = true)]
        projects: Vec<String>,
    },
    /// Scopes: one lens over every view (ADR 0035).
    #[command(subcommand)]
    Scope(scopes::ScopeCommand),
    /// Clocking (ADR 0046): start, stop, add and list clocks; the report.
    #[command(subcommand)]
    Clock(clock::ClockCommand),
    /// The report (GET /report, ADR 0080): what was done, what was worked
    /// on and the time over days, filtered with the search language.
    Report(report::ReportArgs),
    /// Calendars read from elsewhere (ADR 0047): add, set, source,
    /// refresh, delete, list.
    #[command(subcommand)]
    Calendar(calendar::CalendarCommand),
    /// Appointments of a writable calendar (ADR 0068): add, edit, delete.
    #[command(subcommand)]
    Appointment(appointment::AppointmentCommand),
    /// Appointments of some days, repetitions unfolded by the server
    /// (GET /appointments, ADR 0048).
    Appointments {
        /// The first day (YYYY-MM-DD); today by default.
        #[arg(long)]
        from: Option<String>,
        /// The last day, included; `--from` by default.
        #[arg(long)]
        to: Option<String>,
        /// Through this scope instead of the one chosen here (ADR 0035).
        #[arg(long, conflicts_with = "no_scope")]
        scope: Option<String>,
        /// Without a scope, whatever is chosen here.
        #[arg(long)]
        no_scope: bool,
    },
    /// Reminders of tasks and appointments that go off in a window
    /// (GET /alarms, ADR 0101): at most seven days.
    Alarms {
        /// From this time (RFC 3339) or day (YYYY-MM-DD); now by default.
        #[arg(long)]
        from: Option<String>,
        /// Up to this time or day; a day after `--from` by default.
        #[arg(long)]
        to: Option<String>,
        /// Without the reminders of tasks and projects.
        #[arg(long)]
        no_tasks: bool,
        /// Without the reminders of appointments.
        #[arg(long)]
        no_appointments: bool,
    },
    /// Forecast as the server computes it (GET /forecast, ADR 0045): overdue,
    /// today, flagged, the next days, later; each row with why it is there.
    Forecast {
        /// Through this scope instead of the one chosen here (ADR 0035).
        #[arg(long, conflicts_with = "no_scope")]
        scope: Option<String>,
        /// Without a scope, whatever is chosen here.
        #[arg(long)]
        no_scope: bool,
    },
    /// The account's settings (ADR 0045 §4); `set <key> <value>` changes one.
    Settings {
        #[command(subcommand)]
        command: Option<forecast::SettingsCommand>,
    },
    /// Follow changes as they happen (GET /events); one line per event.
    Events {
        /// Stop after this many events.
        #[arg(long)]
        count: Option<usize>,
    },
    /// Send raw changes, a JSON array or one per line, from a file or `-`
    /// (POST /changes).
    Apply {
        file: String,
        /// Let the server stamp them, as for online writes.
        #[arg(long)]
        stamp_server: bool,
    },
    /// Hooks a sender outside samu delivers to, like the Index ring
    /// (ADR 0010, ADR 0022).
    #[command(subcommand)]
    Hook(HookCommand),
    /// The whole account as a directory of JSON files, with the files of
    /// attachments under `files/` (GET /export, ADR 0074). Files that did
    /// not change are left alone; readable without samu.
    Export {
        dir: std::path::PathBuf,
        /// Also write `samu.org`, the tasks as an Org outline to read.
        #[arg(long)]
        org: bool,
    },
    /// Write `samu.org` from an export directory, without a server
    /// (ADR 0074 §8); `--output -` prints it.
    ExportOrg {
        dir: std::path::PathBuf,
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,
    },
    /// Bring tasks in from elsewhere (ADR 0002 §4, ADR 0051).
    #[command(subcommand)]
    Import(ImportCommand),
    /// One action on many tasks, in one write (ADR 0052 §7): done, drop,
    /// reopen, flag, unflag, move, tag, state, date, delete. `-` reads the
    /// ids from stdin, so `samu list --format tsv | samu bulk done -` works.
    #[command(subcommand)]
    Bulk(bulk::BulkCommand),
}

#[derive(Debug, Subcommand)]
enum ImportCommand {
    /// The open tasks of an Org agenda directory: inbox.org to the inbox,
    /// other files to projects, `:ID:` as `org_id`. Runs as often as
    /// wanted, also with other directories: what the account has from an
    /// earlier run (by `org_id` or `org_origin`) stays as it is.
    Org {
        dir: std::path::PathBuf,
        /// Print the changes, one JSON per line, instead of sending them;
        /// `samu apply --stamp-server -` takes them as they are.
        #[arg(long)]
        dry_run: bool,
        /// The zone of times in the files; the system's by default.
        #[arg(long)]
        zone: Option<String>,
        /// Leave this tag out, also where `#+FILETAGS:` sets it; repeatable.
        #[arg(long = "drop-tag", value_name = "TAG")]
        drop_tags: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum HookCommand {
    /// Make a hook; its secret is printed once (POST /hooks).
    Add {
        name: String,
        /// How the hook reads a request (its format; `--format` is the
        /// output's); `index-v1` is the Pebble app.
        #[arg(long, default_value = "index-v1")]
        kind: String,
    },
    /// The hooks of this account (GET /hooks).
    List,
    /// Remove a hook; what it made stays (POST /hooks).
    Remove { name: String },
    /// Give a hook a new secret; the old one stops at once (POST /hooks).
    Secret { name: String },
    /// Deliver as the Pebble app would, to try a hook without the ring
    /// (POST /hooks/{name}).
    Send {
        name: String,
        /// The hook's secret.
        #[arg(long, env = "SAMU_HOOK_SECRET", hide_env_values = true)]
        secret: String,
        /// The transcript; without it, a recording without text.
        #[arg(long)]
        text: Option<String>,
        /// The delivery id; the same one twice makes one task.
        #[arg(long)]
        delivery: Option<String>,
        /// When it was recorded (RFC 3339); now by default.
        #[arg(long)]
        at: Option<String>,
        /// A test event: checked, nothing made.
        #[arg(long)]
        test: bool,
    },
}

/// Which command covers which route. A test holds this against
/// `samu_api::ROUTES` (ADR 0019 §7, catastr ADR 0038).
#[cfg(test)]
const COVERAGE: &[(&str, &str, &str)] = &[
    ("GET", "/health", "health"),
    ("POST", "/login", "login"),
    ("POST", "/logout", "logout"),
    ("GET", "/session", "session"),
    ("GET", "/sessions", "sessions"),
    ("POST", "/sessions/{id}/end", "session end"),
    ("POST", "/changes", "apply"),
    ("GET", "/changes", "changes"),
    ("GET", "/objects/{id}", "show"),
    ("GET", "/objects/{id}/logbook", "log"),
    ("POST", "/objects/{id}/delete", "delete"),
    ("GET", "/available", "list"),
    ("GET", "/blocks", "list"),
    ("GET", "/available/projects", "projects"),
    ("GET", "/available/folders", "folder list"),
    ("GET", "/scopes/scheduled", "scope now"),
    ("GET", "/forecast", "forecast"),
    ("GET", "/alarms", "alarms"),
    ("GET", "/clocks/report", "clock report"),
    ("GET", "/report", "report"),
    ("GET", "/habits", "habit list"),
    ("GET", "/habits/{id}", "habit show"),
    ("POST", "/capture", "add"),
    ("POST", "/render", "render"),
    ("GET", "/events", "events"),
    ("POST", "/pair/code", "pair"),
    ("POST", "/pair", "login"),
    ("GET", "/register", "register"),
    ("POST", "/register", "register"),
    ("POST", "/account/delete", "account delete"),
    ("POST", "/demo", "demo"),
    ("GET", "/hooks", "hook list"),
    ("POST", "/hooks", "hook add"),
    ("POST", "/hooks/{name}", "hook send"),
    ("GET", "/export", "export"),
    ("PUT", "/blobs/{sha256}", "attach"),
    ("GET", "/blobs/{sha256}", "blob get"),
    ("PUT", "/calendars/{id}/source", "calendar source"),
    ("POST", "/calendars/{id}/refresh", "calendar refresh"),
    ("POST", "/calendars/discover", "calendar discover"),
    ("POST", "/calendars/{id}/discover", "calendar discover"),
    ("POST", "/calendars/{id}/import", "calendar import"),
    ("GET", "/appointments", "appointments"),
    ("POST", "/appointments/{id}/edit", "appointment edit"),
];

fn signed_in(config: &Config) -> Result<Client> {
    let token = config.token()?.ok_or_else(|| {
        Failure::Unauthorized(format!(
            "not signed in to {}: run `samu login`",
            config.account.as_deref().unwrap_or("an account")
        ))
    })?;
    Ok(Client::new(config.server()?, Some(token))?)
}

fn password(from_stdin: bool) -> Result<String> {
    if from_stdin {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        return Ok(line.trim_end_matches(['\r', '\n']).to_string());
    }
    Ok(rpassword::prompt_password("password: ")?)
}

/// What `samu account` does.
#[derive(Debug, Subcommand)]
enum AccountCommand {
    /// Delete this account with everything in it, for good, after its
    /// password once more (POST /account/delete, ADR 0099 §5). Only an
    /// account made with `samu register`.
    Delete {
        /// Read the password from the first line of stdin.
        #[arg(long)]
        password_stdin: bool,
    },
}

/// What `samu session` does besides showing the current one.
#[derive(Debug, Subcommand)]
enum SessionCommand {
    /// End a session of the account by its id from `samu sessions`, or the
    /// start of it (POST /sessions/{id}/end, ADR 0095 §7).
    End {
        /// The id, or enough of its start to name one session.
        id: String,
    },
}

/// `POST /login`: the session the server hands out.
fn sign_in_session(config: &Config, login: &Login) -> Result<SessionInfo> {
    let answer = Client::new(config.server()?, None)?.call(
        "POST",
        "/login",
        Some(&serde_json::to_value(login)?),
    )?;
    Ok(serde_json::from_value(answer.json()?)?)
}

/// The token of a fresh session, kept nowhere.
fn sign_in(config: &Config, login: &Login) -> Result<String> {
    sign_in_session(config, login)?
        .token
        .ok_or_else(|| anyhow::anyhow!("the server signed in without handing out a token"))
}

fn access_word(access: samu_api::Access) -> &'static str {
    match access {
        samu_api::Access::Rw => "read-write",
        samu_api::Access::Ro => "read-only",
        samu_api::Access::Unknown => "unknown",
    }
}

fn session_record(session: &SessionInfo) -> Value {
    json!({
        // ADR 0095 §8: none for a session of the whole account.
        "access": session.grant.as_ref().map(|g| g.access),
        "only": session.grant.as_ref().map(|g| g.objects.clone()),
        "account": session.account,
        "device": session.device,
        "kind": session.kind.unwrap_or_default(),
        "expires": session.expires.to_string(),
        "zone": session.zone,
        "due_soon": session.due_soon.map(|p| p.0.to_string()),
        "remind_time": session.remind_time.map(|t| t.to_string()),
        // A registered demo account goes then unless used (ADR 0099 §4).
        "demo_until": session.demo_until.map(|d| d.to_string()),
        // Only from the replica on a computer (ADR 0066 §6).
        "replica": session.replica.as_ref().map(|r| r.upstream.clone()),
        "online": session.replica.as_ref().map(|r| r.online),
        "outbox": session.replica.as_ref().map(|r| r.outbox),
        "last_error": session.replica.as_ref().and_then(|r| r.last_error.clone()),
    })
}

/// A QR code in block characters, dark on light whatever the terminal's
/// colours (as catastr's CLI).
fn qr_for_terminal(text: &str) -> Result<String> {
    use qrcode::render::unicode::Dense1x2;

    let code = qrcode::QrCode::new(text.as_bytes())?;
    let drawn = code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Dark)
        .light_color(Dense1x2::Light)
        .quiet_zone(true)
        .build();
    Ok(drawn
        .lines()
        .map(|line| format!("\x1b[30;107m{line}\x1b[0m"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// A stamp the server replaces (ADR 0019 §4).
fn placeholder() -> Hlc {
    Hlc::new(0, 0, "cli")
}

/// The last eight characters, enough to tell objects apart in a list.
fn short(id: &Id) -> String {
    let full = id.to_string();
    full[full.len().saturating_sub(8)..].to_string()
}

/// An id from a full id, a `samu:` URI, or the end of one, like git's short
/// hashes.
fn resolve(client: &Client, reference: &str) -> Result<Id> {
    if let Ok(id) = reference.parse::<Id>() {
        return Ok(id);
    }
    if let Some(id) = Id::from_uri(reference) {
        return Ok(id);
    }
    let wanted = reference.to_ascii_lowercase();
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let matches: Vec<Id> = page
        .objects
        .iter()
        .filter_map(AnyObject::id)
        .filter(|id| id.to_string().ends_with(&wanted))
        .collect();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(Failure::NotFound(format!("nothing matches {reference:?}")).into()),
        many => bail!(
            "{reference:?} matches {} objects: {}",
            many.len(),
            many.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// An object as [`resolve`] finds it, else an open project by title, as
/// `samu review` takes it: `samu set Keller review_interval P2W`.
fn resolve_or_project(client: &Client, reference: &str) -> Result<Id> {
    match resolve(client, reference) {
        Err(error) if matches!(error.downcast_ref::<Failure>(), Some(Failure::NotFound(_))) => {
            resolve_project(client, reference)
        }
        found => found,
    }
}

/// `POST /objects/{id}/delete` (ADR 0036).
fn delete_object(client: &Client, id: Id, request: &DeleteRequest) -> Result<DeleteResponse> {
    let answer = client.call(
        "POST",
        &format!("/objects/{id}/delete"),
        Some(&serde_json::to_value(request)?),
    )?;
    Ok(serde_json::from_value(answer.json()?)?)
}

fn delete_record(answer: &DeleteResponse) -> Value {
    json!({
        "deleted": answer.deleted,
        "moved": answer.moved,
        "untagged": answer.untagged,
        "seq": answer.seq,
    })
}

/// Sends changes the server stamps, and fails unless all are applied.
fn write(client: &Client, changes: &[Change]) -> Result<Vec<ChangeOutcome>> {
    let answer = client.call(
        "POST",
        "/changes?stamp=server",
        Some(&serde_json::to_value(changes)?),
    )?;
    let outcomes: Vec<ChangeOutcome> = serde_json::from_value(answer.json()?)?;
    for outcome in &outcomes {
        match outcome {
            ChangeOutcome::Applied { .. } => {}
            ChangeOutcome::Rejected { reason } => bail!("rejected: {reason}"),
            other => bail!("not applied: {}", serde_json::to_string(other)?),
        }
    }
    Ok(outcomes)
}

fn set_field(client: &Client, id: Id, field: &str, value: Value) -> Result<()> {
    set_noted(client, id, field, value, None)
}

/// `set` with a note for the logbook (ADR 0024 §5).
fn set_noted(
    client: &Client,
    id: Id,
    field: &str,
    value: Value,
    note: Option<String>,
) -> Result<()> {
    write(
        client,
        &[Change::Set {
            object: id,
            field: field.into(),
            value,
            note,
            hlc: placeholder(),
        }],
    )?;
    Ok(())
}

/// Says on stderr which task a completed repetition made (ADR 0033): the
/// server makes it in the same change, so it is among what changed since.
fn report_next(client: &Client, completed: Id, seq: u64) -> Result<()> {
    let answer = client.call(
        "GET",
        &format!("/changes?after={}", seq.saturating_sub(1)),
        None,
    )?;
    let page: ChangesPage = serde_json::from_value(answer.json()?)?;
    for object in &page.objects {
        if let AnyObject::Task(task) = object
            && task.repeat_of == Some(completed)
        {
            let when = [("due", &task.schedule.due), ("defer", &task.schedule.defer)]
                .into_iter()
                .find_map(|(name, when)| when.as_ref().map(|w| format!(" {name} {}", when_text(w))))
                .unwrap_or_default();
            eprintln!("next: {} {}{when}", short(&task.id), task.title);
        }
    }
    Ok(())
}

/// One line of a list: kind, short id, title.
fn object_record(object: &AnyObject) -> Value {
    let value = serde_json::to_value(object).unwrap_or(Value::Null);
    let inner = &value["object"];
    json!({
        "kind": value["kind"],
        "id": inner["id"],
        "short": object.id().map(|id| short(&id)),
        "title": inner.get("title").or_else(|| inner.get("uri")).cloned().unwrap_or(Value::Null),
        "object": inner,
    })
}

/// What `list` shows besides the tasks: titles of projects, tags and
/// tasks, and the tag tree for paths.
struct Names {
    projects: std::collections::HashMap<Id, String>,
    tags: std::collections::HashMap<Id, samu_api::Tag>,
    tasks: std::collections::HashMap<Id, String>,
}

impl Names {
    fn new(page: &ChangesPage, statuses: &[TaskStatus]) -> Self {
        let mut names = Self {
            projects: std::collections::HashMap::new(),
            tags: std::collections::HashMap::new(),
            tasks: statuses
                .iter()
                .map(|s| (s.task.id, s.task.title.clone()))
                .collect(),
        };
        for object in &page.objects {
            match object {
                AnyObject::Project(p) => {
                    names.projects.insert(p.id, p.title.clone());
                }
                AnyObject::Tag(t) => {
                    names.tags.insert(t.id, t.clone());
                }
                _ => {}
            }
        }
        names
    }

    /// `ort/büro`.
    fn tag_path(&self, id: Id) -> String {
        let mut parts = Vec::new();
        let mut at = self.tags.get(&id);
        while let Some(tag) = at {
            parts.push(tag.title.clone());
            if parts.len() > 16 {
                break;
            }
            at = tag.parent.and_then(|p| self.tags.get(&p));
        }
        parts.reverse();
        parts.join("/")
    }

    /// `id` and every tag below it.
    fn tag_and_below(&self, id: Id) -> std::collections::HashSet<Id> {
        let mut out = std::collections::HashSet::from([id]);
        loop {
            let before = out.len();
            for tag in self.tags.values() {
                if tag.parent.is_some_and(|p| out.contains(&p)) {
                    out.insert(tag.id);
                }
            }
            if out.len() == before {
                return out;
            }
        }
    }

    fn task(&self, id: Id) -> String {
        self.tasks.get(&id).cloned().unwrap_or_else(|| short(&id))
    }

    fn reason(&self, reason: &samu_api::Reason) -> String {
        use samu_api::Reason as R;
        match reason {
            R::Completed => "completed".into(),
            R::Dropped => "dropped".into(),
            R::Deferred { until } => format!("deferred until {}", when_text(until)),
            R::ProjectNotActive { project } => format!(
                "project on hold: {}",
                self.projects.get(project).cloned().unwrap_or_default()
            ),
            R::FolderDropped { .. } => "folder dropped".into(),
            R::TagNotActive { tag } => format!("tag on hold: {}", self.tag_path(*tag)),
            R::WaitingInSequence { first } => format!("waits for: {}", self.task(*first)),
            R::HasOpenChildren => "group: children first".into(),
            R::ContainerUnavailable { .. } => "the group above is not available".into(),
            R::BlockedBy { task } => format!("blocked by: {}", self.task(*task)),
            R::UnknownValue => "a value this samu does not know".into(),
            R::Cycle => "a cycle in the data".into(),
            R::Unknown => "a reason from a newer server".into(),
        }
    }
}

/// `2026-11-13` or `2026-11-13 09:00`.
pub(crate) fn when_text(when: &samu_api::When) -> String {
    let value = serde_json::to_value(when).unwrap_or(Value::Null);
    if let Some(date) = value["date"].as_str() {
        return date.to_string();
    }
    if let Some(at) = value["at"].as_str() {
        return at.get(..16).unwrap_or(at).replace('T', " ");
    }
    String::new()
}

/// One line of `list`.
/// What `samu list` narrows on this side: a tag and those below it,
/// flagged, and words in title, note or id.
fn narrow(
    statuses: &mut Vec<TaskStatus>,
    names: &Names,
    tag: Option<&str>,
    flagged: bool,
    search: Option<&str>,
) -> Result<()> {
    if let Some(tag) = tag {
        let wanted = tag.trim_start_matches('@').to_lowercase();
        let hits: Vec<Id> = names
            .tags
            .values()
            .filter(|t| {
                t.title.to_lowercase() == wanted || names.tag_path(t.id).to_lowercase() == wanted
            })
            .map(|t| t.id)
            .collect();
        if hits.is_empty() {
            return Err(Failure::NotFound(format!("no tag {tag:?}")).into());
        }
        let mut set = std::collections::HashSet::new();
        for hit in hits {
            set.extend(names.tag_and_below(hit));
        }
        statuses.retain(|s| s.task.tags.iter().any(|t| set.contains(&t.tag)));
    }
    if flagged {
        statuses.retain(|s| s.task.flagged);
    }
    if let Some(search) = search {
        let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
        statuses.retain(|s| {
            // The id too, so its end finds the task (as the web).
            let text = format!(
                "{}\n{}\n{}",
                s.task.title,
                s.task.note.as_deref().unwrap_or_default(),
                s.task.id
            )
            .to_lowercase();
            words.iter().all(|w| text.contains(w))
        });
    }
    Ok(())
}

/// The sections of a block perspective (ADR 0116 §7): in plain a heading
/// before each block's rows, in tsv the heading as the first column, in
/// json and jsonl the sections with their tasks.
fn print_blocks(
    client: &Client,
    format: Format,
    sections: Vec<BlockSection>,
    tag: Option<&str>,
    flagged: bool,
    search: Option<&str>,
) -> Result<()> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let every: Vec<TaskStatus> = sections.iter().flat_map(|s| s.tasks.clone()).collect();
    let names = Names::new(&page, &every);
    const COLUMNS: [&str; 8] = [
        "short", "st", "flag", "due_text", "project", "title", "tags", "why",
    ];
    let mut out = Vec::new();
    for (n, mut section) in sections.into_iter().enumerate() {
        narrow(&mut section.tasks, &names, tag, flagged, search)?;
        let mut records: Vec<Value> = section
            .tasks
            .iter()
            .map(|s| status_record(s, &names))
            .collect();
        match format {
            Format::Plain => {
                if n > 0 {
                    println!();
                }
                let icon = section
                    .icon
                    .as_deref()
                    .map(|i| format!("{i} "))
                    .unwrap_or_default();
                let missing = if section.missing { " — missing" } else { "" };
                println!("▸ {icon}{} ({}){missing}", section.title, records.len());
                emit(format, &records, &COLUMNS)?;
            }
            Format::Tsv => {
                for record in &mut records {
                    record["block"] = json!(section.title);
                }
                let mut columns = vec!["block"];
                columns.extend(COLUMNS);
                emit(format, &records, &columns)?;
            }
            Format::Json | Format::Jsonl => out.push(json!({
                "title": section.title,
                "icon": section.icon,
                "block": section.block,
                "missing": section.missing,
                "tasks": records,
            })),
        }
    }
    if matches!(format, Format::Json | Format::Jsonl) {
        emit(format, &out, &[])?;
    }
    Ok(())
}

fn status_record(status: &TaskStatus, names: &Names) -> Value {
    let task = &status.task;
    let reasons: Vec<String> = status
        .reasons
        .iter()
        .map(|r| {
            serde_json::to_value(r)
                .ok()
                .and_then(|v| v["reason"].as_str().map(str::to_string))
                .unwrap_or_default()
        })
        .collect();
    let state = status.state.and_then(|s| names.tags.get(&s));
    let due_state = serde_json::to_value(status.due).unwrap_or(Value::Null);
    let due = task.schedule.due.as_ref().map(|due| {
        let mut text = when_text(due);
        match due_state.as_str() {
            Some("overdue") => text.push_str(" overdue"),
            Some("soon") => text.push_str(" soon"),
            _ => {}
        }
        text
    });
    let tags: Vec<String> = task
        .tags
        .iter()
        .filter(|t| Some(t.tag) != status.state)
        .map(|t| format!("@{}", names.tag_path(t.tag)))
        .collect();
    json!({
        "id": task.id,
        "short": short(&task.id),
        "title": task.title,
        "available": status.available,
        "due": due_state,
        "today": status.today,
        "reasons": reasons.join(","),
        "state": state.map(|t| t.title.clone()).unwrap_or_default(),
        "st": state.map(|t| t.key.clone().unwrap_or_else(|| t.title.chars().take(3).collect())).unwrap_or_default(),
        "flag": if task.flagged { "!" } else { "" },
        "priority": task.priority,
        "due_text": due.unwrap_or_default(),
        "project": task.project.and_then(|p| names.projects.get(&p)).cloned().unwrap_or_default(),
        "tags": tags.join(" "),
        "why": status.reasons.first().map(|r| names.reason(r)).unwrap_or_default(),
        "next_repeat": next_repeat_text(status),
        "status": status,
    })
}

/// When the next task of a repetition would come: its leading date, or
/// `ended`; empty for a task that does not repeat.
fn next_repeat_text(status: &TaskStatus) -> String {
    let Some(next) = &status.next_repeat else {
        return String::new();
    };
    if next.ended {
        return "ended".into();
    }
    [&next.due, &next.defer, &next.planned]
        .into_iter()
        .flatten()
        .next()
        .map(when_text)
        .unwrap_or_default()
}

/// A project by id, `samu:` URI, title (exact, else a unique start, any
/// case) or the end of its id; open projects first for titles.
fn resolve_project(client: &Client, reference: &str) -> Result<Id> {
    if let Ok(id) = reference.parse::<Id>() {
        return Ok(id);
    }
    if let Some(id) = Id::from_uri(reference) {
        return Ok(id);
    }
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let open: Vec<&samu_api::Project> = page
        .objects
        .iter()
        .filter_map(|o| match o {
            AnyObject::Project(p)
                if matches!(
                    p.status,
                    samu_api::ProjectStatus::Active | samu_api::ProjectStatus::OnHold
                ) =>
            {
                Some(p)
            }
            _ => None,
        })
        .collect();
    let wanted = reference.to_lowercase();
    if let Some(p) = open.iter().find(|p| p.title.to_lowercase() == wanted) {
        return Ok(p.id);
    }
    let started: Vec<&&samu_api::Project> = open
        .iter()
        .filter(|p| p.title.to_lowercase().starts_with(&wanted))
        .collect();
    match started.as_slice() {
        [one] => Ok(one.id),
        [] => resolve(client, reference),
        many => bail!(
            "{reference:?} starts {} projects: {}",
            many.len(),
            many.iter()
                .map(|p| p.title.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The answer of `POST /capture`: what it made or would make on stderr,
/// the task on stdout. `written`: whether it was written, not a preview.
fn print_capture(
    format: Format,
    captured: &CaptureResponse,
    written: bool,
    columns: &[&str],
) -> Result<()> {
    for name in &captured.new_tags {
        if written {
            eprintln!("made the tag {name}");
        } else {
            eprintln!("would make the tag {name}");
        }
    }
    for warning in &captured.warnings {
        eprintln!("warning: {}", warning_text(warning));
    }
    for reminder in &captured.reminders {
        let verb = if written { "reminds" } else { "would remind" };
        let when = remind::record(reminder, None)["when"].clone();
        eprintln!("{verb} {}", when.as_str().unwrap_or_default());
    }
    let task = &captured.task;
    let record = json!({
        "id": task.id,
        "short": short(&task.id),
        "title": task.title,
        "created": captured.created,
        "task": task,
        "new_tags": captured.new_tags,
        "warnings": captured.warnings,
    });
    emit(format, &[record], columns)
}

/// A capture warning in words, for stderr (ADR 0027 §6).
pub(crate) fn warning_text(warning: &CaptureWarning) -> String {
    let what = |meaning: &samu_api::CaptureMeaning| {
        serde_json::to_value(meaning)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    };
    match warning {
        CaptureWarning::UnknownDate { meaning, text } => {
            format!(
                "{text}: no {} date I know; it stays in the title",
                what(meaning)
            )
        }
        CaptureWarning::UnknownEstimate { text } => {
            format!("{text}: no length of time; it stays in the title")
        }
        CaptureWarning::UnknownRepeat { text } => {
            format!("{text}: no repetition I know; it stays in the title")
        }
        CaptureWarning::UnknownHabit { text } => {
            format!("{text}: no habit I know (habit:, 3/w, 8/d:Glas, avoid); it stays in the title")
        }
        CaptureWarning::Twice { meaning, text } => {
            format!("{text}: a second {}; it stays in the title", what(meaning))
        }
        CaptureWarning::UnknownProject { name } => {
            format!("#{name}: no project, or several; the task goes to the inbox")
        }
        CaptureWarning::UnknownTag { name } => {
            format!("@{name}: several tags, or a new one under a workflow; it stays in the title")
        }
        CaptureWarning::UnknownTemplate { name } => {
            format!("/{name}: no template; it stays in the title")
        }
        CaptureWarning::FreeText { text } => {
            format!("{text:?}: no mark; an edit takes none, the title has its own field")
        }
        CaptureWarning::ClockInFuture { text } => {
            format!("{text}: it would end in the future; not booked")
        }
        CaptureWarning::UnknownClock { text } => {
            format!("{text}: no time I know; it stays in the title")
        }
        CaptureWarning::Unknown => "a warning from a newer server".to_string(),
    }
}

/// One line of `projects`.
fn project_record(entry: &ProjectAvailability) -> Value {
    let reasons: Vec<String> = entry
        .reasons
        .iter()
        .map(|r| {
            serde_json::to_value(r)
                .ok()
                .and_then(|v| v["reason"].as_str().map(str::to_string))
                .unwrap_or_default()
        })
        .collect();
    json!({
        "id": entry.project.id,
        "short": short(&entry.project.id),
        "title": entry.project.title,
        "status": serde_json::to_value(entry.project.status).unwrap_or(Value::Null),
        "available": entry.available,
        "stuck": if entry.stuck { "stuck" } else { "" },
        "review": entry.review.map(|d| d.to_string()).unwrap_or_default(),
        "review_due": if entry.review_due { "due" } else { "" },
        "reasons": reasons.join(","),
        "project": entry,
    })
}

fn read_changes(file: &str) -> Result<Vec<Change>> {
    let mut text = String::new();
    if file == "-" {
        std::io::stdin().read_to_string(&mut text)?;
    } else {
        text = std::fs::read_to_string(file)?;
    }
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') {
        return Ok(serde_json::from_str(trimmed)?);
    }
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

fn run(args: Args) -> Result<()> {
    let config = Config::load(args.server, args.account, args.profile)?;
    match args.command {
        Command::Health => {
            let answer = Client::new(config.server()?, None)?.call("GET", "/health", None)?;
            emit(
                args.format,
                &[answer.json()?],
                &["status", "version", "api"],
            )?;
        }
        Command::Login {
            code: Some(code), ..
        } => {
            let answer = Client::new(config.server()?, None)?.call(
                "POST",
                "/pair",
                Some(&serde_json::to_value(PairRequest { code })?),
            )?;
            let session: SessionInfo = serde_json::from_value(answer.json()?)?;
            let Some(token) = &session.token else {
                bail!("the server paired without handing out a token");
            };
            if session.account != config.account()? {
                bail!(
                    "the code is for {}, not {}: set the account first",
                    session.account,
                    config.account()?
                );
            }
            config.save_token(token)?;
            eprintln!("paired with {} as {}", session.account, session.device);
            emit(
                args.format,
                &[session_record(&session)],
                &["account", "device", "expires"],
            )?;
        }
        Command::Pair {
            device,
            password_stdin,
        } => {
            let client = signed_in(&config)?;
            let request = PairCodeRequest {
                device,
                password: password(password_stdin)?,
            };
            let answer =
                client.call("POST", "/pair/code", Some(&serde_json::to_value(&request)?))?;
            let code: PairCode = serde_json::from_value(answer.json()?)?;
            if args.format == Format::Plain && std::io::stdout().is_terminal() {
                println!("{}", qr_for_terminal(&code.uri)?);
            }
            eprintln!(
                "valid until {} and once; scan it in the app, or `samu login --code`",
                code.expires
            );
            emit(
                args.format,
                &[json!({
                    "uri": code.uri,
                    "code": code.code,
                    "account": code.account,
                    "device": code.device,
                    "expires": code.expires.to_string(),
                })],
                &["uri", "code"],
            )?;
        }
        Command::Login {
            password_stdin,
            device,
            agent,
            only,
            read_only,
            code: None,
        } => {
            let account = config.account()?.to_string();
            let mut login = Login {
                account: account.clone(),
                password: password(password_stdin)?,
                device: device.unwrap_or_else(|| config.device.clone()),
                kind: agent.then_some(samu_api::AuthorKind::Agent),
                only: None,
            };
            if !only.is_empty() {
                let objects = match grant::ids_only(&only) {
                    Some(ids) => ids,
                    // Paths need a session to look them up (ADR 0095 §8):
                    // a short one of the whole account, ended right after.
                    None => {
                        let looking = sign_in(&config, &login)?;
                        let client = Client::new(config.server()?, Some(looking))?;
                        let found = grant::resolve_all(&client, &only);
                        client.call("POST", "/logout", None)?;
                        found?
                    }
                };
                login.only = Some(samu_api::Grant {
                    objects,
                    access: if read_only {
                        samu_api::Access::Ro
                    } else {
                        samu_api::Access::Rw
                    },
                });
            }
            let session = sign_in_session(&config, &login)?;
            let Some(token) = &session.token else {
                bail!("the server signed in without handing out a token");
            };
            config.save_token(token)?;
            let kind = match session.kind {
                Some(samu_api::AuthorKind::Agent) => " (agent)",
                _ => "",
            };
            let grant = match &session.grant {
                Some(g) => format!(", only {} ({})", only.join(", "), access_word(g.access)),
                None => String::new(),
            };
            eprintln!("signed in to {account} as {}{kind}{grant}", session.device);
            emit(
                args.format,
                &[session_record(&session)],
                &["account", "device", "expires"],
            )?;
        }
        Command::Logout => {
            let client = signed_in(&config)?;
            // A session already gone on the server is fine: forgetting the
            // token here is all that is left to do. Any other failure is not.
            if let Err(error) = client.call("POST", "/logout", None)
                && !matches!(error, Failure::Unauthorized(_))
            {
                return Err(error.into());
            }
            config.forget_token()?;
            eprintln!("signed out of {}", config.account()?);
        }
        Command::Register { info: true, .. } => {
            let answer = Client::new(config.server()?, None)?.call("GET", "/register", None)?;
            let info: samu_api::RegistrationInfo = serde_json::from_value(answer.json()?)?;
            emit(
                args.format,
                &[json!({"mode": info.mode, "languages": info.languages.join(",")})],
                &["mode", "languages"],
            )?;
        }
        Command::Register {
            name,
            password_stdin,
            demo,
            device,
            info: false,
        } => {
            let request = samu_api::Register {
                password: match &name {
                    Some(_) => Some(password(password_stdin)?),
                    None => None,
                },
                name,
                demo,
                device: device.unwrap_or_else(|| config.device.clone()),
            };
            let answer = Client::new(config.server()?, None)?.call(
                "POST",
                "/register",
                Some(&serde_json::to_value(&request)?),
            )?;
            let made: samu_api::Registered = serde_json::from_value(answer.json()?)?;
            match config.for_account(&made.account) {
                Some(theirs) => {
                    theirs.save_token(&made.token)?;
                    eprintln!(
                        "account {} made and signed in; use it with --account {} or SAMU_ACCOUNT",
                        made.account, made.account
                    );
                }
                None => eprintln!(
                    "account {} made; the profile is another account's, so sign in to it \
                     with `samu --account {} login`",
                    made.account, made.account
                ),
            }
            if made.password.is_some() {
                eprintln!("note the password: it is shown this once");
            }
            emit(
                args.format,
                &[json!({
                    "account": made.account,
                    "password": made.password,
                    "expires": made.expires.to_string(),
                })],
                &["account", "password", "expires"],
            )?;
        }
        Command::Demo { lang } => {
            let request = samu_api::DemoRequest { lang };
            let answer = signed_in(&config)?.call(
                "POST",
                "/demo",
                Some(&serde_json::to_value(&request)?),
            )?;
            let added: samu_api::DemoAdded = serde_json::from_value(answer.json()?)?;
            if added.whole {
                eprintln!("demo data added to the whole account");
            } else {
                eprintln!("demo data added under the folder and the tag Demo");
            }
            let rows: Vec<Value> = added
                .counts
                .iter()
                .map(|(kind, count)| json!({"kind": kind, "count": count}))
                .collect();
            emit(args.format, &rows, &["kind", "count"])?;
        }
        Command::Account(AccountCommand::Delete { password_stdin }) => {
            let client = signed_in(&config)?;
            let request = samu_api::AccountDelete {
                password: password(password_stdin)?,
            };
            client.call(
                "POST",
                "/account/delete",
                Some(&serde_json::to_value(&request)?),
            )?;
            config.forget_token()?;
            eprintln!(
                "account {} deleted, with everything in it",
                config.account()?
            );
        }
        Command::Render { file, read_as } => {
            let text = if file == "-" {
                let mut text = String::new();
                std::io::stdin().lock().read_to_string(&mut text)?;
                text
            } else {
                std::fs::read_to_string(&file).with_context(|| format!("reading {file}"))?
            };
            let request = json!({"texts": [{"text": text, "format": read_as}]});
            let answer = signed_in(&config)?.call("POST", "/render", Some(&request))?;
            let rendered: RenderResponse = serde_json::from_value(answer.json()?)?;
            let records: Vec<Value> = rendered
                .documents
                .iter()
                .map(|document| json!({"format": document.format, "document": document}))
                .collect();
            emit(args.format, &records, &["format"])?;
        }
        Command::Session {
            action: Some(SessionCommand::End { id }),
        } => {
            signed_in(&config)?.call("POST", &format!("/sessions/{}/end", id.trim()), None)?;
            eprintln!("ended session {}", id.trim());
        }
        Command::Sessions => {
            let answer = signed_in(&config)?.call("GET", "/sessions", None)?;
            let sessions: Vec<samu_api::SessionSummary> = serde_json::from_value(answer.json()?)?;
            let records: Vec<Value> = sessions
                .iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "device": s.device,
                        "kind": s.kind,
                        "created": s.created.to_string(),
                        "expires": s.expires.to_string(),
                        "access": s.grant.as_ref().map(|g| g.access),
                        "only": s.grant.as_ref().map(|g| g.objects.clone()),
                        "current": s.current,
                    })
                })
                .collect();
            emit(
                args.format,
                &records,
                &["id", "device", "kind", "access", "expires", "current"],
            )?;
        }
        Command::Session { action: None } => {
            let client = signed_in(&config)?;
            let answer = client.call("GET", "/session", None)?;
            let session: SessionInfo = serde_json::from_value(answer.json()?)?;
            let mut record = session_record(&session);
            if let Some(grant) = &session.grant {
                record["grant"] = json!(format!(
                    "{}: {}",
                    access_word(grant.access),
                    grant::titles(&client, &grant.objects)?.join(", ")
                ));
            }
            emit(
                args.format,
                &[record],
                &["account", "device", "expires", "zone", "grant"],
            )?;
        }
        Command::Add {
            title,
            project,
            parent,
            note,
            flag,
            dry_run,
            literal: false,
            no_scope,
            template,
        } => {
            let client = signed_in(&config)?;
            let project = project.map(|p| resolve_project(&client, &p)).transpose()?;
            let parent = parent.map(|p| resolve(&client, &p)).transpose()?;
            let mut text = title.join(" ");
            if let Some(template) = template {
                let template = template.trim_start_matches('/');
                if !templates::callable(template) {
                    bail!("{template:?} is no template's name: letters, digits, - and _");
                }
                // Before a note, which takes the rest of the line.
                text = match text.split_once(" // ") {
                    Some((head, note)) => format!("{head} /{template} // {note}"),
                    None => format!("{text} /{template}"),
                };
            }
            if flag {
                text.push_str(" !");
            }
            if let Some(note) = note {
                if text.split_whitespace().any(|word| word == "//") {
                    bail!("the text holds a note after // already; leave out --note");
                }
                text.push_str(" // ");
                text.push_str(&note);
            }
            let request = CaptureRequest {
                text,
                project,
                parent,
                // The scope chosen here fills in what the text names not
                // (ADR 0043 §6); the server ignores one that is gone.
                scope: if no_scope {
                    None
                } else {
                    scopes::effective_scope(&client, &config)?
                },
                template: None,
                task: None,
                preview: dry_run,
            };
            let answer = client.call("POST", "/capture", Some(&serde_json::to_value(&request)?))?;
            let captured: CaptureResponse = serde_json::from_value(answer.json()?)?;
            let columns: &[&str] = if captured.created {
                &["id"]
            } else {
                &["title"]
            };
            print_capture(args.format, &captured, captured.created, columns)?;
        }
        Command::Edit { id, line, preview } => {
            let client = signed_in(&config)?;
            let task = resolve(&client, &id)?;
            let request = CaptureRequest {
                text: line.join(" "),
                project: None,
                parent: None,
                scope: None,
                template: None,
                task: Some(task),
                preview,
            };
            let answer = client.call("POST", "/capture", Some(&serde_json::to_value(&request)?))?;
            let edited: CaptureResponse = serde_json::from_value(answer.json()?)?;
            print_capture(args.format, &edited, !preview, &["short", "title"])?;
        }
        Command::Add {
            title,
            project,
            note,
            flag,
            literal: true,
            ..
        } => {
            let client = signed_in(&config)?;
            let project = project.map(|p| resolve_project(&client, &p)).transpose()?;
            let id = Id::new();
            let now = jiff::Timestamp::now().to_string();
            let mut task = json!({
                "id": id,
                "created": now,
                "modified": now,
                "title": title.join(" "),
                "flagged": flag,
                "sequential": false,
                "complete_with_last_child": false,
                "position": 0,
            });
            if let Some(project) = project {
                task["project"] = json!(project);
            }
            if let Some(note) = note {
                task["note"] = json!(note);
            }
            write(
                &client,
                &[Change::Create {
                    object: id,
                    kind: ObjectKind::Task,
                    value: task,
                    hlc: placeholder(),
                }],
            )?;
            emit(
                args.format,
                &[json!({"id": id, "short": short(&id)})],
                &["id"],
            )?;
        }
        Command::Set { id, field, value } => {
            let client = signed_in(&config)?;
            let id = resolve_or_project(&client, &id)?;
            let value = if field == "own_scope" {
                // By title or the end of an id, like everywhere (ADR 0094 §8).
                json!(scopes::scope_named(&client, &value)?)
            } else {
                serde_json::from_str(&value).unwrap_or(Value::String(value))
            };
            set_field(&client, id, &field, value)?;
        }
        Command::Unset { id, field } => {
            let client = signed_in(&config)?;
            let id = resolve_or_project(&client, &id)?;
            set_field(&client, id, &field, Value::Null)?;
        }
        Command::Done { id, note } => {
            let client = signed_in(&config)?;
            let id = resolve(&client, &id)?;
            let outcomes = write(
                &client,
                &[Change::Set {
                    object: id,
                    field: "completed_at".into(),
                    value: json!(jiff::Timestamp::now().to_string()),
                    note,
                    hlc: placeholder(),
                }],
            )?;
            if let Some(ChangeOutcome::Applied { seq }) = outcomes.first() {
                report_next(&client, id, *seq)?;
            }
        }
        Command::Drop { id, note } => {
            let client = signed_in(&config)?;
            let id = resolve(&client, &id)?;
            set_noted(
                &client,
                id,
                "dropped_at",
                json!(jiff::Timestamp::now().to_string()),
                note,
            )?;
        }
        Command::State {
            id,
            state,
            next,
            none,
            note,
        } => {
            let client = signed_in(&config)?;
            let id = resolve(&client, &id)?;
            let target = if next {
                tags::Target::Next
            } else if none {
                tags::Target::None
            } else if let Some(state) = state {
                tags::Target::Named(state)
            } else {
                bail!("name a state, or give --next or --none");
            };
            let shown = tags::set_state(&client, id, &target, note)?;
            emit(args.format, &[shown], &["state"])?;
        }
        Command::Export { dir, org } => {
            let client = signed_in(&config)?;
            export::export(&client, &dir, org)?;
        }
        Command::ExportOrg { dir, output } => export::export_org(&dir, output.as_ref())?,
        Command::Log { id } => {
            let client = signed_in(&config)?;
            let id = resolve(&client, &id)?;
            let answer = client.call("GET", &format!("/objects/{id}/logbook"), None)?;
            let entries: Vec<LogEntry> = serde_json::from_value(answer.json()?)?;
            let records: Vec<Value> = entries
                .iter()
                .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
                .collect();
            emit(
                args.format,
                &records,
                &["at", "kind", "device", "old", "new", "note"],
            )?;
        }
        Command::Tag(command) => tags::run(&config, args.format, command)?,
        Command::Bulk(command) => bulk::run(&config, args.format, command)?,
        Command::Scope(command) => scopes::run(&config, args.format, command)?,
        Command::Profile(command) => profiles::run(&config, args.format, command)?,
        Command::Template(command) => templates::run(&config, args.format, command)?,
        Command::Habit(command) => habits::run(&config, args.format, command)?,
        Command::Perspective(command) => perspectives::run(&config, args.format, command)?,
        Command::Clock(command) => clock::run(&config, args.format, command)?,
        Command::Report(report) => report::run(&config, args.format, &report)?,
        Command::Calendar(command) => calendar::run(&config, args.format, command)?,
        Command::Appointment(command) => appointment::run(&config, args.format, command)?,
        Command::Appointments {
            from,
            to,
            scope,
            no_scope,
        } => calendar::appointments(
            &config,
            args.format,
            from.as_deref(),
            to.as_deref(),
            scope.as_deref(),
            no_scope,
        )?,
        Command::Alarms {
            from,
            to,
            no_tasks,
            no_appointments,
        } => calendar::alarms(
            &config,
            args.format,
            (from.as_deref(), to.as_deref()),
            !no_tasks,
            !no_appointments,
        )?,
        Command::Forecast { scope, no_scope } => {
            forecast::run(&config, args.format, scope.as_deref(), no_scope)?;
        }
        Command::Settings { command } => forecast::settings(&config, args.format, command)?,
        Command::Remind(remind) => {
            let client = signed_in(&config)?;
            let owner = resolve(&client, &remind.id)?;
            let reminder = remind::reminder(owner, &remind)?;
            write(&client, &[remind::create(&reminder, placeholder())?])?;
            emit(args.format, &[remind::record(&reminder, None)], &["id"])?;
        }
        Command::Block { id, blocker } => {
            blockers::block(&signed_in(&config)?, args.format, &id, &blocker)?;
        }
        Command::Unblock { id, blocker } => {
            blockers::unblock(&signed_in(&config)?, &id, &blocker)?;
        }
        Command::Blockers { id } => {
            blockers::list(&signed_in(&config)?, args.format, &id)?;
        }
        Command::Comment { id, text } => {
            comments::comment(&signed_in(&config)?, args.format, &id, &text)?;
        }
        Command::Comments { id } => {
            comments::comments(&signed_in(&config)?, args.format, &id)?;
        }
        Command::Attach(attach) => {
            comments::attach(&signed_in(&config)?, args.format, &attach)?;
        }
        Command::Attachments { id } => {
            comments::attachments(&signed_in(&config)?, args.format, &id)?;
        }
        Command::Blob(command) => {
            comments::blob(&signed_in(&config)?, &command)?;
        }
        Command::Reminders { id } => {
            let client = signed_in(&config)?;
            let owner = id.map(|id| resolve(&client, &id)).transpose()?;
            let records = remind::list(&client, owner)?;
            emit(args.format, &records, &["short", "when", "title"])?;
        }
        Command::Delete { id, tasks, dry_run } => {
            let client = signed_in(&config)?;
            let id = resolve(&client, &id)?;
            let tasks = match tasks.as_deref() {
                None => None,
                Some("inbox") => Some(TaskFate::Inbox),
                Some("delete") => Some(TaskFate::Delete),
                Some(project) => Some(TaskFate::Project(resolve_project(&client, project)?)),
            };
            let answer = delete_object(
                &client,
                id,
                &DeleteRequest {
                    tasks,
                    preview: dry_run,
                },
            )?;
            emit(
                args.format,
                &[delete_record(&answer)],
                &["deleted", "moved", "untagged"],
            )?;
        }
        Command::Project(command) => places::project(&config, args.format, command)?,
        Command::Folder(command) => places::folder(&config, args.format, command)?,
        Command::Prop { id } => props::show(&config, args.format, &id)?,
        Command::Open { id, key, print } => props::open(&config, args.format, &id, &key, print)?,
        Command::OpenUri { uri, print, notify } => {
            props::open_uri(&config, args.format, &uri, print, notify)?;
        }
        Command::Show { id } => {
            let client = signed_in(&config)?;
            let id = resolve(&client, &id)?;
            let answer = client.call("GET", &format!("/objects/{id}"), None)?;
            let object: AnyObject = serde_json::from_value(answer.json()?)?;
            if args.format == Format::Plain {
                println!("{}", serde_json::to_string_pretty(&object)?);
            } else {
                emit(
                    args.format,
                    &[object_record(&object)],
                    &["kind", "id", "title"],
                )?;
            }
        }
        Command::Changes { after } => {
            let client = signed_in(&config)?;
            let answer = client.call("GET", &format!("/changes?after={after}"), None)?;
            let page: ChangesPage = serde_json::from_value(answer.json()?)?;
            let mut records: Vec<Value> = page.objects.iter().map(object_record).collect();
            records.extend(
                page.gone
                    .iter()
                    .map(|id| json!({"kind": "gone", "id": id, "short": short(id), "title": null})),
            );
            emit(args.format, &records, &["kind", "short", "title"])?;
            eprintln!("current to change {}", page.seq);
        }
        Command::List {
            available,
            today,
            project,
            all,
            tag,
            flagged,
            search,
            scope,
            no_scope,
            perspective,
            bare,
            columns,
        } => {
            let client = signed_in(&config)?;
            let mut query = vec![];
            let mut columns = columns.as_deref().map(table::parse).transpose()?;
            let scoped = scopes::query_scope(&client, &config, scope.as_deref(), no_scope)?;
            if let Some(scope) = scoped {
                query.push(format!("scope={scope}"));
            }
            if let Some(perspective) = perspective {
                let found = perspectives::resolve(&client, &perspective)?;
                if !found.blocks.is_empty() {
                    if available || today || all || bare || project.is_some() || columns.is_some() {
                        bail!(
                            "a block perspective shows its blocks as they are: no --available, \
                             --today, --all, --project, --bare or --columns"
                        );
                    }
                    let mut path = format!("/blocks?perspective={}", found.id);
                    if let Some(scope) = scoped {
                        path.push_str(&format!("&scope={scope}"));
                    }
                    let sections: Vec<BlockSection> =
                        serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
                    return print_blocks(
                        &client,
                        args.format,
                        sections,
                        tag.as_deref(),
                        flagged,
                        search.as_deref(),
                    );
                }
                if columns.is_none() && found.view == samu_api::PerspectiveView::Table {
                    columns = Some(found.table.map(|t| t.columns).unwrap_or_default());
                }
                query.push(format!("perspective={}", found.id));
            }
            if available {
                query.push("available=true".to_string());
            }
            if today {
                query.push("today=true".to_string());
            }
            if all {
                query.push("all=true".to_string());
            }
            if let Some(project) = project {
                query.push(format!("project={}", resolve_project(&client, &project)?));
            }
            let path = if query.is_empty() {
                "/available".to_string()
            } else {
                format!("/available?{}", query.join("&"))
            };
            if bare {
                let path = format!(
                    "{path}{}bare=true",
                    if query.is_empty() { "?" } else { "&" }
                );
                let statuses: Vec<BareStatus> =
                    serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
                let records = statuses
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<Value>, _>>()?;
                emit(args.format, &records, &["id", "available", "due", "today"])?;
                return Ok(());
            }
            let mut statuses: Vec<TaskStatus> =
                serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
            // Due outside the scope: a hint, never a row (ADR 0035 §6).
            let outside = statuses.iter().filter(|s| s.outside).count();
            statuses.retain(|s| !s.outside);
            if outside > 0 {
                // Without a scope, what has an own scope is the hidden
                // (ADR 0069 §7, ADR 0094 §4).
                if scoped.is_some() {
                    eprintln!("{outside} due outside the scope");
                } else {
                    eprintln!("{outside} due, hidden: shown only in its own scope");
                }
            }
            let page: ChangesPage =
                serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
            let names = Names::new(&page, &statuses);
            narrow(
                &mut statuses,
                &names,
                tag.as_deref(),
                flagged,
                search.as_deref(),
            )?;
            let mut records: Vec<Value> =
                statuses.iter().map(|s| status_record(s, &names)).collect();
            if let Some(columns) = columns {
                let table =
                    table::Table::new(table::or_default(columns), &page, jiff::Timestamp::now());
                if matches!(args.format, Format::Json | Format::Jsonl) {
                    for (record, status) in records.iter_mut().zip(&statuses) {
                        record["columns"] = table.values(status);
                    }
                    emit(args.format, &records, &[])?;
                } else {
                    let mut lines: Vec<Value> = Vec::new();
                    if args.format == Format::Plain && !statuses.is_empty() {
                        lines.push(table.head());
                    }
                    lines.extend(statuses.iter().map(|s| table.line(&short(&s.task.id), s)));
                    emit(args.format, &lines, &table.keys())?;
                }
                return Ok(());
            }
            emit(
                args.format,
                &records,
                &[
                    "short", "st", "flag", "due_text", "project", "title", "tags", "why",
                ],
            )?;
        }
        Command::Projects {
            stuck,
            review,
            all,
            scope,
            no_scope,
        } => {
            let client = signed_in(&config)?;
            let mut query = vec![];
            if stuck {
                query.push("stuck=true".to_string());
            }
            if review {
                query.push("review=true".to_string());
            }
            if all {
                query.push("all=true".to_string());
            }
            if let Some(scope) = scopes::query_scope(&client, &config, scope.as_deref(), no_scope)?
            {
                query.push(format!("scope={scope}"));
            }
            let path = if query.is_empty() {
                "/available/projects".to_string()
            } else {
                format!("/available/projects?{}", query.join("&"))
            };
            let projects: Vec<ProjectAvailability> =
                serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
            let records: Vec<Value> = projects.iter().map(project_record).collect();
            emit(
                args.format,
                &records,
                &["short", "status", "stuck", "review", "title", "reasons"],
            )?;
        }
        Command::Review { projects } => {
            let client = signed_in(&config)?;
            let now = json!(jiff::Timestamp::now().to_string());
            let mut changes = Vec::new();
            for reference in &projects {
                let id = resolve_project(&client, reference)?;
                for (field, value) in [("last_reviewed", now.clone()), ("next_review", Value::Null)]
                {
                    changes.push(Change::Set {
                        object: id,
                        field: field.into(),
                        value,
                        note: None,
                        hlc: placeholder(),
                    });
                }
            }
            write(&client, &changes)?;
            eprintln!("reviewed: {}", projects.len());
        }
        Command::Events { count } => {
            let client = signed_in(&config)?;
            let mut seen = 0;
            client.events("/events", |event, data| {
                let data: Value = serde_json::from_str(data).unwrap_or(Value::String(data.into()));
                let record = json!({"event": event, "data": data});
                if args.format == Format::Plain {
                    println!("{event}  {data}");
                } else {
                    emit(args.format, &[record], &["event", "data"])?;
                }
                use std::io::Write as _;
                std::io::stdout().flush()?;
                seen += 1;
                Ok::<_, anyhow::Error>(count.is_none_or(|count| seen < count))
            })?;
        }
        Command::Apply { file, stamp_server } => {
            let client = signed_in(&config)?;
            let changes = read_changes(&file)?;
            let path = if stamp_server {
                "/changes?stamp=server"
            } else {
                "/changes"
            };
            let answer = client.call("POST", path, Some(&serde_json::to_value(&changes)?))?;
            let outcomes: Vec<Value> = serde_json::from_value(answer.json()?)?;
            emit(args.format, &outcomes, &["outcome", "seq", "reason"])?;
            if outcomes.iter().any(|o| o["outcome"] != "applied") {
                bail!("not every change was applied");
            }
        }
        Command::Hook(command) => hook(&config, args.format, command)?,
        Command::Import(ImportCommand::Org {
            dir,
            dry_run,
            zone,
            drop_tags,
        }) => {
            import_org(
                &config,
                args.format,
                &dir,
                dry_run,
                zone.as_deref(),
                &drop_tags,
            )?;
        }
    }
    Ok(())
}

fn hook_record(hook: &HookView) -> Value {
    json!({"name": hook.name, "format": hook.format, "created": hook.created})
}

fn hook_change(client: &Client, change: &HookChange) -> Result<HookChangeResponse> {
    let answer = client.call("POST", "/hooks", Some(&serde_json::to_value(change)?))?;
    Ok(serde_json::from_value(answer.json()?)?)
}

fn hook(config: &Config, format: Format, command: HookCommand) -> Result<()> {
    let fields = &["name", "format", "created", "secret"];
    match command {
        HookCommand::Add { name, kind } => {
            let kind: HookFormat = serde_json::from_value(Value::String(kind.clone()))?;
            if kind == HookFormat::Unknown {
                bail!("{kind:?} is not a hook format; known is index-v1");
            }
            let client = signed_in(config)?;
            let response = hook_change(&client, &HookChange::Add { name, format: kind })?;
            print_secret(format, &response, fields)?;
        }
        HookCommand::Secret { name } => {
            let client = signed_in(config)?;
            let response = hook_change(&client, &HookChange::Secret { name })?;
            print_secret(format, &response, fields)?;
        }
        HookCommand::Remove { name } => {
            let client = signed_in(config)?;
            hook_change(&client, &HookChange::Remove { name: name.clone() })?;
            eprintln!("removed the hook {name}; what it made stays");
        }
        HookCommand::List => {
            let answer = signed_in(config)?.call("GET", "/hooks", None)?;
            let hooks: Vec<HookView> = serde_json::from_value(answer.json()?)?;
            let records: Vec<Value> = hooks.iter().map(hook_record).collect();
            emit(format, &records, &["name", "format", "created"])?;
        }
        HookCommand::Send {
            name,
            secret,
            text,
            delivery,
            at,
            test,
        } => {
            let recorded = match at {
                Some(at) => at.parse::<jiff::Timestamp>()?,
                None => jiff::Timestamp::now(),
            };
            let delivery = ring::Delivery {
                delivery: delivery.unwrap_or_else(|| format!("samu-send-{}", Id::new())),
                text,
                recorded_at: recorded.as_millisecond(),
                test,
            };
            let (content_type, headers, body) =
                ring::request(&secret, &delivery, jiff::Timestamp::now().as_second());
            // No session: the signature is the sign-in.
            let client = Client::new(config.server()?, None)?;
            let answer =
                client.call_raw(&format!("/hooks/{name}"), &content_type, &headers, &body)?;
            let response: HookDelivery = serde_json::from_value(answer.json()?)?;
            emit(
                format,
                &[json!({
                    "task": response.task,
                    "short": response.task.as_ref().map(short),
                    "created": response.created,
                    "test": response.test,
                })],
                &["short", "created", "test"],
            )?;
        }
    }
    Ok(())
}

/// A hook with its secret on stdout, and on stderr that it is shown once.
fn print_secret(format: Format, response: &HookChangeResponse, fields: &[&str]) -> Result<()> {
    let Some(hook) = &response.hook else {
        bail!("the server answered without the hook");
    };
    let mut record = hook_record(hook);
    record["secret"] = json!(response.secret);
    emit(format, &[record], fields)?;
    eprintln!(
        "the secret is shown this once; enter it in the sender (Pebble app: \
         \"Sign requests\" for the gesture)"
    );
    Ok(())
}

fn import_org(
    config: &Config,
    format: Format,
    dir: &std::path::Path,
    dry_run: bool,
    zone: Option<&str>,
    drop_tags: &[String],
) -> Result<()> {
    let zone = match zone {
        Some(name) => jiff::tz::TimeZone::get(name)?,
        None => jiff::tz::TimeZone::system(),
    };
    let files = import::files_of(dir)?;
    let client = signed_in(config)?;
    // A server before properties would drop org_id and source without a
    // word, and the guard below would never see a run (ADR 0023).
    let health = client.call("GET", "/health", None)?.json()?;
    let api = health["api"].as_str().unwrap_or("");
    if !api_has_properties(api) {
        bail!("the server speaks samu-api {api:?}; the import needs 0.1.3 or newer");
    }
    let answer = client.call("GET", "/changes?after=0", None)?;
    let page: ChangesPage = serde_json::from_value(answer.json()?)?;
    let existing = import::Existing::from_objects(&page.objects);
    // The directory's own name tells its files from those of another one
    // (ADR 0051 §2): inbox.org stands in several.
    let name = std::fs::canonicalize(dir)?
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let plan = import::plan(
        &files,
        &name,
        &zone,
        jiff::Timestamp::now(),
        &existing,
        drop_tags,
    );
    for message in &plan.messages {
        eprintln!("{message}");
    }
    eprintln!(
        "{} files: {} projects, {} tasks ({} with :ID:), {} new tags, {} known",
        files.len(),
        plan.projects,
        plan.tasks,
        plan.with_org_id,
        plan.tags,
        plan.known
    );
    if dry_run {
        for change in &plan.changes {
            println!("{}", serde_json::to_string(change)?);
        }
        return Ok(());
    }
    if let Err(error) = write(&client, &plan.changes) {
        bail!(
            "{error:#}; what came before it is in the account, with source={}",
            import::SOURCE
        );
    }
    emit(
        format,
        &[json!({
            "projects": plan.projects,
            "tasks": plan.tasks,
            "with_org_id": plan.with_org_id,
            "tags": plan.tags,
            "known": plan.known,
        })],
        &["projects", "tasks", "with_org_id", "tags", "known"],
    )?;
    Ok(())
}

/// Whether a samu-api version (`major.minor.patch`) has properties: 0.1.3
/// or newer.
fn api_has_properties(version: &str) -> bool {
    let parts: Vec<u64> = version.split('.').map_while(|p| p.parse().ok()).collect();
    match parts.as_slice() {
        [major, minor, patch] => (*major, *minor, *patch) >= (0, 1, 3),
        _ => false,
    }
}

/// The exit code for a failure, from its cause rather than its message.
fn exit_code(error: &anyhow::Error) -> u8 {
    // A profile that is not there or not well written (ADR 0086).
    if error
        .downcast_ref::<samu_client::profile::ProfileError>()
        .is_some()
    {
        return exit::USAGE;
    }
    match error.downcast_ref::<Failure>() {
        Some(Failure::NoServer(_)) => exit::NO_SERVER,
        Some(Failure::NotFound(_)) => exit::NOT_FOUND,
        Some(Failure::Unauthorized(_)) => exit::UNAUTHORIZED,
        Some(_) | None => exit::FAILURE,
    }
}

fn main() -> ExitCode {
    // With COMPLETE=zsh (or bash, fish) set by the shell: print completions
    // and exit; otherwise nothing happens here.
    clap_complete::CompleteEnv::with_factory(Args::command).complete();
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::from(exit::SUCCESS),
        Err(error) => {
            eprintln!("samu: {error:#}");
            ExitCode::from(exit_code(&error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every route has a command, and every listed command is real
    /// (ADR 0019 §7).
    #[test]
    fn every_route_has_a_command() {
        use clap::CommandFactory;
        let cli = Args::command();
        for route in samu_api::ROUTES {
            let covered = COVERAGE
                .iter()
                .find(|(method, path, _)| *method == route.method && *path == route.path);
            let Some((_, _, command)) = covered else {
                panic!("{} {} has no command", route.method, route.path);
            };
            let found = command
                .split(' ')
                .try_fold(&cli, |cmd, name| cmd.find_subcommand(name));
            assert!(
                found.is_some(),
                "{command} is listed for {} {} but does not exist",
                route.method,
                route.path
            );
        }
        assert_eq!(
            COVERAGE.len(),
            samu_api::ROUTES.len(),
            "a command for a route that is gone"
        );
    }

    #[test]
    fn the_import_needs_a_server_with_properties() {
        assert!(!api_has_properties("0.1.2"));
        assert!(api_has_properties("0.1.3"));
        assert!(api_has_properties("0.2.0"));
        assert!(api_has_properties("1.0.0"));
        assert!(!api_has_properties(""));
        assert!(!api_has_properties("0.1"));
    }

    #[test]
    fn exit_codes_follow_the_cause() {
        assert_eq!(
            exit_code(&Failure::NoServer("x".into()).into()),
            exit::NO_SERVER
        );
        assert_eq!(
            exit_code(&Failure::NotFound("x".into()).into()),
            exit::NOT_FOUND
        );
        assert_eq!(
            exit_code(&Failure::Unauthorized("x".into()).into()),
            exit::UNAUTHORIZED
        );
        assert_eq!(exit_code(&anyhow::anyhow!("x")), exit::FAILURE);
        assert_eq!(
            exit_code(&samu_client::profile::ProfileError::Unknown("x".into()).into()),
            exit::USAGE
        );
        assert_eq!(exit::USAGE, 2);
    }
}
