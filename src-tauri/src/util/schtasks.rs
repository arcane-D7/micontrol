//! Single port for Windows Task Scheduler operations (`schtasks.exe`).
//!
//! Why this exists (architecture governor C1b / U4 + L-01): every
//! `schtasks` spawn used to live beside the feature that needed it
//! (`elev_bridge` ran/re-registered the elevation task, `hw::sys_opt`
//! toggled CEIP tasks), each with its own console-window handling, its own
//! output parsing and its own error strings. This module is the ONLY place
//! allowed to spawn `schtasks` (gate `scripts/check-schtasks-calls.mjs`,
//! allowlist empty), and all operations go through
//! [`crate::util::process_spawn::run_capture`]: mandatory timeout, tree kill
//! on expiry, `CREATE_NO_WINDOW`, 1 MiB stream cap.
//!
//! Parsing lives here too: verbose CSV (`/fo csv /v`) and task XML are
//! decoded by pure functions in this file, so callers never see raw
//! scheduler text. Errors are catalogued [`AppError`]s built from
//! `ErrorCode::{TaskQueryFailed, TaskControlFailed, TaskConfigFailed,
//! TaskAccessDenied}` (never a bare String).
//!
//! L-01 note: a missing task is a query-level fact, not a control failure.
//! `exists()` answers it cheaply, `is_not_found()` classifies the error, and
//! the [`gate`] submodule holds the session-state machine that keeps the
//! elevation task from being re-created per command (see `elev_bridge`).
//!
//! All operations are synchronous: callers on async paths must wrap them in
//! `util::fixed_pool::run_blocking_timeout` with a timeout, like every other
//! spawn-bound helper.

use crate::errors::catalog::{AppError, ErrorCode};
use crate::util::process_spawn::{self, SpawnOutput};
use std::time::Duration;

/// Every `schtasks` invocation through this port is bounded by this budget.
/// The scheduler answers in tens of milliseconds; 15 s covers a cold,
/// loaded machine without approaching any caller budget.
const SCHTASKS_TIMEOUT: Duration = Duration::from_secs(15);

/// The scheduled task that carries elevated commands (installer-registered,
/// `RunLevel=HighestAvailable`, interactive token). Single definition so the
/// name can never drift between the dispatcher and the self-heal path.
pub const ELEVATED_TASK_NAME: &str = "MiControlElevated";

// ---------------------------------------------------------------------------
// Spec types (create via command line)
// ---------------------------------------------------------------------------

/// How the task runs: with the limited user token or the highest available
/// token (the elevation task needs `HighestAvailable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunLevel {
    /// Default token.
    Limited,
    /// `HIGHEST` run level.
    HighestAvailable,
}

/// Logon type: interactive token (console user, no stored password) or a
/// service-style account (`/ru` only, no `/it`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogonType {
    /// Interactive token: the task only runs for the logged-on user (`/it`).
    InteractiveToken,
    /// Run whether the user is logged on or not; requires [`TaskSpec::user`].
    Batch,
}

/// Everything `/create` needs besides the task name.
#[derive(Debug, Clone)]
pub struct TaskSpec {
    /// Program to execute (`/tr` command part).
    pub command: String,
    /// Program arguments, one per argument (`/tr` remainder, joined).
    pub args: Vec<String>,
    /// Account the task runs as (`/ru`); `None` = current user.
    pub user: Option<String>,
    /// Token level (`/rl`).
    pub run_level: RunLevel,
    /// Logon type (`/it` vs batch).
    pub logon_type: LogonType,
}

impl Default for TaskSpec {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            user: None,
            run_level: RunLevel::Limited,
            logon_type: LogonType::InteractiveToken,
        }
    }
}

/// Observed scheduler state for one task, from verbose CSV output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskState {
    /// Scheduler status string (`Ready`, `Running`, ...).
    pub status: String,
    /// Exit code of the last run (decimal; negative HRESULTs preserved).
    pub last_result: i64,
    /// Last run timestamp exactly as the scheduler reported it.
    pub last_run: String,
}

// ---------------------------------------------------------------------------
// Public operations (each = one spawn through process_spawn)
// ---------------------------------------------------------------------------

/// Create a task from a [`TaskSpec`] (command-line `/create`).
///
/// `/f` overwrites an existing registration with the same name.
#[cfg(windows)]
pub fn create(name: &str, spec: &TaskSpec) -> Result<(), AppError> {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.arg("/create")
        .arg("/tn")
        .arg(name)
        .arg("/f")
        .arg("/rl")
        .arg(match spec.run_level {
            RunLevel::Limited => "LIMITED",
            RunLevel::HighestAvailable => "HIGHEST",
        });
    if spec.logon_type == LogonType::InteractiveToken {
        cmd.arg("/it");
    }
    if let Some(user) = &spec.user {
        cmd.arg("/ru").arg(user);
    }
    let mut tr = format!("\"{}\"", spec.command);
    if !spec.args.is_empty() {
        tr.push(' ');
        tr.push_str(&spec.args.join(" "));
    }
    cmd.arg("/tr").arg(tr);
    let out = process_spawn::run_capture(&mut cmd, SCHTASKS_TIMEOUT)
        .map_err(|e| spawn_err(ErrorCode::TaskConfigFailed, "create", name, &e))?;
    ensure_success("create", name, &out)
}

/// Create (or overwrite) a task from a full task XML document.
///
/// The XML is written to a private temp file, handed to
/// `schtasks /create /tn <name> /xml <file> /f`, and deleted afterwards.
#[cfg(windows)]
pub fn create_from_xml(name: &str, xml: &str) -> Result<(), AppError> {
    let file = write_temp_xml(name, xml)?;
    let mut cmd = std::process::Command::new("schtasks");
    cmd.arg("/create")
        .arg("/tn")
        .arg(name)
        .arg("/xml")
        .arg(&file)
        .arg("/f");
    let result = process_spawn::run_capture(&mut cmd, SCHTASKS_TIMEOUT)
        .map_err(|e| spawn_err(ErrorCode::TaskConfigFailed, "create", name, &e))
        .and_then(|out| ensure_success("create", name, &out));
    let _ = std::fs::remove_file(&file);
    result
}

/// Delete a task registration. Missing task = success (idempotent), so
/// cleanup paths never need a pre-query.
#[cfg(windows)]
pub fn delete(name: &str) -> Result<(), AppError> {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.arg("/delete").arg("/tn").arg(name).arg("/f");
    let out = process_spawn::run_capture(&mut cmd, SCHTASKS_TIMEOUT)
        .map_err(|e| spawn_err(ErrorCode::TaskConfigFailed, "delete", name, &e))?;
    if out.success() {
        return Ok(());
    }
    if classify_output(&combined(&out)) == FailKind::NotFound {
        return Ok(());
    }
    Err(control_err("delete", name, &out))
}

/// Start a task now (`/run`). The scheduler returns immediately; the task
/// itself runs asynchronously.
#[cfg(windows)]
pub fn run(name: &str) -> Result<(), AppError> {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.arg("/run").arg("/tn").arg(name);
    let out = process_spawn::run_capture(&mut cmd, SCHTASKS_TIMEOUT)
        .map_err(|e| spawn_err(ErrorCode::TaskControlFailed, "run", name, &e))?;
    ensure_success("run", name, &out)
}

/// Stop a running task (`/end`).
#[cfg(windows)]
pub fn end(name: &str) -> Result<(), AppError> {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.arg("/end").arg("/tn").arg(name);
    let out = process_spawn::run_capture(&mut cmd, SCHTASKS_TIMEOUT)
        .map_err(|e| spawn_err(ErrorCode::TaskControlFailed, "end", name, &e))?;
    ensure_success("end", name, &out)
}

/// Enable or disable a task (`/change /enable|/disable`).
#[cfg(windows)]
pub fn set_enabled(name: &str, enabled: bool) -> Result<(), AppError> {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.arg("/change")
        .arg("/tn")
        .arg(name)
        .arg(if enabled { "/enable" } else { "/disable" });
    let out = process_spawn::run_capture(&mut cmd, SCHTASKS_TIMEOUT)
        .map_err(|e| spawn_err(ErrorCode::TaskControlFailed, "change", name, &e))?;
    ensure_success("change", name, &out)
}

/// Query a task's state as structured [`TaskState`] (verbose CSV).
#[cfg(windows)]
pub fn query(name: &str) -> Result<TaskState, AppError> {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.arg("/query")
        .arg("/tn")
        .arg(name)
        .arg("/fo")
        .arg("csv")
        .arg("/v");
    let out = process_spawn::run_capture(&mut cmd, SCHTASKS_TIMEOUT)
        .map_err(|e| spawn_err(ErrorCode::TaskQueryFailed, "query", name, &e))?;
    if !out.success() {
        return Err(query_err("query", name, &out));
    }
    parse_query_csv(&out.stdout_str()).ok_or_else(|| {
        AppError::new(
            ErrorCode::TaskQueryFailed,
            format!("query({name}): scheduler returned no parsable CSV row"),
        )
    })
}

/// Query a task's definition XML (raw document, as registered).
#[cfg(windows)]
pub fn query_xml(name: &str) -> Result<String, AppError> {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.arg("/query").arg("/tn").arg(name).arg("/xml");
    let out = process_spawn::run_capture(&mut cmd, SCHTASKS_TIMEOUT)
        .map_err(|e| spawn_err(ErrorCode::TaskQueryFailed, "query_xml", name, &e))?;
    if !out.success() {
        return Err(query_err("query_xml", name, &out));
    }
    Ok(out.stdout_str())
}

/// True when the scheduler has a registration for `name`.
///
/// A query that fails for any OTHER reason also answers `false`: callers
/// treat `false` as "use the primary path", which is always the safe
/// direction. Do not use this where the distinction matters; use
/// [`query`] and inspect the error instead.
#[cfg(windows)]
pub fn exists(name: &str) -> bool {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.arg("/query")
        .arg("/tn")
        .arg(name)
        .arg("/fo")
        .arg("csv")
        .arg("/nh");
    let out = process_spawn::run_capture(&mut cmd, SCHTASKS_TIMEOUT);
    match out {
        Ok(out) => out.success() && !out.stdout_str().trim().is_empty(),
        Err(_) => false,
    }
}

// Non-Windows: every operation is unsupported on this platform.

#[cfg(not(windows))]
pub fn unsupported() -> AppError {
    AppError::new(ErrorCode::NotSupported, "schtasks requires Windows")
}

#[cfg(not(windows))]
pub fn create(_name: &str, _spec: &TaskSpec) -> Result<(), AppError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn create_from_xml(_name: &str, _xml: &str) -> Result<(), AppError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn delete(_name: &str) -> Result<(), AppError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn run(_name: &str) -> Result<(), AppError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn end(_name: &str) -> Result<(), AppError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn set_enabled(_name: &str, _enabled: bool) -> Result<(), AppError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn query(_name: &str) -> Result<TaskState, AppError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn query_xml(_name: &str) -> Result<String, AppError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn exists(_name: &str) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Error classification (pure, unit-tested)
// ---------------------------------------------------------------------------

/// What the scheduler's combined output says about a failure.
///
/// `schtasks` writes errors to STDOUT, not stderr (S55), and localizes
/// them; match on both the English and pt-PT phrasings, case-insensitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// The task name is not registered on this machine.
    NotFound,
    /// The scheduler refused the operation (standard user, needs elevation).
    AccessDenied,
    /// Anything else.
    Other,
}

/// Classify combined scheduler output.
pub fn classify_output(text: &str) -> FailKind {
    let t = text.to_lowercase();
    if t.contains("access is denied") || t.contains("acesso negado") {
        return FailKind::AccessDenied;
    }
    if t.contains("does not exist")
        || t.contains("não existe")
        || t.contains("cannot find")
        || t.contains("não é possível encontrar")
        || t.contains("no scheduled tasks")
        || t.contains("nenhuma tarefa agendada")
    {
        return FailKind::NotFound;
    }
    FailKind::Other
}

/// The exact context prefix [`is_not_found`] matches on.
pub const NOT_FOUND_PREFIX: &str = "task not found:";

/// Build the canonical "missing task" error for any operation.
fn not_found_err(op: &str, name: &str, detail: &str) -> AppError {
    AppError::new(
        ErrorCode::TaskQueryFailed,
        format!("{NOT_FOUND_PREFIX} {op}({name}): {detail}"),
    )
}

/// True when the error says the task simply is not registered.
pub fn is_not_found(err: &AppError) -> bool {
    err.context.starts_with(NOT_FOUND_PREFIX)
}

fn combined(out: &SpawnOutput) -> String {
    // schtasks reports failures on stdout (S55); keep stderr in the mix for
    // spawn-level text.
    format!("{}{}", out.stdout_str(), out.stderr_str())
}

fn spawn_err(default: ErrorCode, op: &str, name: &str, e: &process_spawn::SpawnError) -> AppError {
    AppError::new(default, format!("{op}({name}): spawn failed: {e}"))
}

fn query_err(op: &str, name: &str, out: &SpawnOutput) -> AppError {
    match classify_output(&combined(out)) {
        FailKind::NotFound => not_found_err(op, name, combined(out).trim()),
        FailKind::AccessDenied => AppError::new(
            ErrorCode::TaskAccessDenied,
            format!("{op}({name}): access denied: {}", combined(out).trim()),
        ),
        FailKind::Other => AppError::new(
            ErrorCode::TaskQueryFailed,
            format!("{op}({name}): {}", combined(out).trim()),
        ),
    }
}

fn control_err(op: &str, name: &str, out: &SpawnOutput) -> AppError {
    match classify_output(&combined(out)) {
        FailKind::NotFound => not_found_err(op, name, combined(out).trim()),
        FailKind::AccessDenied => AppError::new(
            ErrorCode::TaskAccessDenied,
            format!("{op}({name}): access denied: {}", combined(out).trim()),
        ),
        FailKind::Other => AppError::new(
            ErrorCode::TaskControlFailed,
            format!("{op}({name}): {}", combined(out).trim()),
        ),
    }
}

fn ensure_success(op: &str, name: &str, out: &SpawnOutput) -> Result<(), AppError> {
    if out.success() {
        return Ok(());
    }
    match classify_output(&combined(out)) {
        FailKind::NotFound => Err(not_found_err(op, name, combined(out).trim())),
        FailKind::AccessDenied => Err(AppError::new(
            ErrorCode::TaskAccessDenied,
            format!("{op}({name}): access denied: {}", combined(out).trim()),
        )),
        FailKind::Other => {
            let code = match op {
                "create" => ErrorCode::TaskConfigFailed,
                _ => ErrorCode::TaskControlFailed,
            };
            Err(AppError::new(
                code,
                format!("{op}({name}): {}", combined(out).trim()),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers (pure, unit-tested)
// ---------------------------------------------------------------------------

/// Parse one CSV line honouring double quotes and `""` escapes.
pub fn parse_csv_row(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            fields.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    fields.push(cur);
    fields
}

/// Parse verbose CSV (`/fo csv /v`) into a [`TaskState`], matching fields by
/// HEADER NAME (never by position: the scheduler's column order is a
/// compatibility surface, not an API).
pub fn parse_query_csv(stdout: &str) -> Option<TaskState> {
    let mut header: Option<Vec<String>> = None;
    for line in stdout.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let row = parse_csv_row(line);
        match &header {
            None => header = Some(row),
            Some(h) => {
                let idx = |want: &str| h.iter().position(|f| f.trim().to_lowercase() == want);
                let status = row.get(idx("status")?)?.trim().to_string();
                let last_result = idx("last result")
                    .and_then(|i| row.get(i))
                    .and_then(|v| v.trim().parse::<i64>().ok())
                    .unwrap_or(0);
                let last_run = idx("last run time")
                    .and_then(|i| row.get(i))
                    .map(|v| v.trim().to_string())
                    .unwrap_or_default();
                return Some(TaskState {
                    status,
                    last_result,
                    last_run,
                });
            }
        }
    }
    None
}

/// Write `xml` to a private temp file for `/create /xml`.
#[cfg(windows)]
fn write_temp_xml(name: &str, xml: &str) -> Result<String, AppError> {
    let file = std::env::temp_dir().join(format!(
        "micontrol_task_{}_{}.xml",
        name.replace(['\\', '/'], "_"),
        std::process::id()
    ));
    std::fs::write(&file, xml).map_err(|e| {
        AppError::new(
            ErrorCode::TaskConfigFailed,
            format!("create({name}): cannot write task XML temp file: {e}"),
        )
    })?;
    Ok(file.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Elevation task XML (single definition, same document the installer writes)
// ---------------------------------------------------------------------------

/// The `MiControlElevated` task XML pointing at `exe`.
///
/// Interactive token, `HighestAvailable`, `PT120S` execution limit, disabled
/// time trigger (manual start only). Keep byte-compatible with the
/// installer: the self-heal path compares and re-registers this document.
pub fn elevated_task_xml(exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><TimeTrigger><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>false</Enabled></TimeTrigger></Triggers><Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT120S</ExecutionTimeLimit><Enabled>true</Enabled></Settings><Actions Context="Author"><Exec><Command>"{exe}"</Command><Arguments>--elevated</Arguments></Exec></Actions></Task>"#
    )
}

/// True when a task XML's action targets `exe` (any slash direction, with or
/// without surrounding quotes).
///
/// The XML stores `<Command>"C:\...\micontrol.exe"</Command>` WITH quotes and
/// may use forward slashes; comparing against only the raw path made a
/// correct task always look wrong (MIOT-38 heal loop).
pub fn xml_targets_path(xml: &str, exe: &str) -> bool {
    if exe.is_empty() {
        return false;
    }
    let path_quoted = format!("\"{exe}\"");
    let path_quoted_fwd = format!("\"{}\"", exe.replace('\\', "/"));
    xml.contains(exe)
        || xml.contains(&exe.replace('\\', "/"))
        || xml.contains(&path_quoted)
        || xml.contains(&path_quoted_fwd)
}

// ---------------------------------------------------------------------------
// L-01 gate: session state machine for the elevation task path
// ---------------------------------------------------------------------------
//
// The pre-gate behaviour: every elevated command that reached the task path
// ran `schtasks /run`, and on failure re-ran `/query /xml` + `/delete` +
// `/create` (which fails unelevated) and emitted 3 WARN lines. With the task
// missing (standard-user session, task not registered), 146 such lines
// accumulated in 2.5 h. This gate makes the task path:
//   (a) consult `exists` at most once per [`EXISTS_REFRESH_MS`] window;
//   (b) re-CREATE the task at most once per exponential backoff window
//       (1 min, 5 min, 30 min), never per command;
//   (c) report exactly one WARN per missing-task episode, with the running
//       total attached, instead of one per command.
//
// Pure over injected time and existence: tests drive it without spawning
// anything. The process-wide instance lives in `elev_bridge`.

/// How long a cached `exists` answer stays valid before the gate re-queries.
pub const EXISTS_REFRESH_MS: u64 = 60_000;

/// Exponential backoff between re-create attempts (ms).
pub const HEAL_BACKOFF_MS: [u64; 3] = [60_000, 300_000, 1_800_000];

/// What the gate tells the dispatcher to do with the task path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDecision {
    /// The task exists: dispatch through it.
    Attempt,
    /// The task is missing: use the bridge service pipe (the primary
    /// elevated path) and fail fast; `emit_warn` is true exactly once per
    /// episode, `missing_total` is the accumulated count to log.
    Missing { emit_warn: bool, missing_total: u64 },
}

/// Session state for the elevation task path. Owns no handles; all clocks
/// are injected `now_ms` (u64 epoch milliseconds).
#[derive(Debug, Default)]
pub struct TaskGate {
    exists_cache: Option<(bool, u64)>,
    episode_open: bool,
    missing_total: u64,
    heal_attempts: u32,
    next_heal_at_ms: u64,
}

impl TaskGate {
    /// True when the cached `exists` answer is stale or absent.
    pub fn exists_refresh_due(&self, now_ms: u64) -> bool {
        match self.exists_cache {
            Some((_, at)) => now_ms.saturating_sub(at) >= EXISTS_REFRESH_MS,
            None => true,
        }
    }

    /// Store an `exists` answer observed at `now_ms`.
    pub fn set_exists(&mut self, now_ms: u64, exists: bool) {
        self.exists_cache = Some((exists, now_ms));
    }

    /// Decide what to do with the task path right now.
    ///
    /// The caller is responsible for refreshing the `exists` cache
    /// ([`exists_refresh_due`] + [`set_exists`]); a stale cache must never
    /// claim the task exists, so it is treated as missing (fail safe:
    /// a skipped task attempt costs one pipe round-trip, a wrong attempt
    /// re-opens the L-01 loop).
    pub fn decide(&mut self, now_ms: u64) -> TaskDecision {
        let exists = match self.exists_cache {
            Some((e, at)) if now_ms.saturating_sub(at) < EXISTS_REFRESH_MS => e,
            _ => false,
        };
        if exists {
            // A task that reappeared closes the episode.
            if self.episode_open {
                self.episode_open = false;
            }
            return TaskDecision::Attempt;
        }
        self.missing_total = self.missing_total.saturating_add(1);
        let emit_warn = !self.episode_open;
        self.episode_open = true;
        TaskDecision::Missing {
            emit_warn,
            missing_total: self.missing_total,
        }
    }

    /// True when a re-create (heal) attempt is allowed at `now_ms`.
    pub fn heal_attempt_allowed(&self, now_ms: u64) -> bool {
        now_ms >= self.next_heal_at_ms
    }

    /// Record a FAILED heal; the next attempt waits one backoff step.
    pub fn note_heal_failed(&mut self, now_ms: u64) {
        let idx = (self.heal_attempts as usize).min(HEAL_BACKOFF_MS.len() - 1);
        self.next_heal_at_ms = now_ms.saturating_add(HEAL_BACKOFF_MS[idx]);
        self.heal_attempts = self.heal_attempts.saturating_add(1);
        self.set_exists(now_ms, false);
    }

    /// Record a SUCCESSFUL heal (task re-created and visible again).
    pub fn note_healed(&mut self, now_ms: u64) {
        self.heal_attempts = 0;
        self.next_heal_at_ms = 0;
        self.set_exists(now_ms, true);
        self.episode_open = false;
    }

    /// Accumulated missing-task observations this session.
    pub fn missing_total(&self) -> u64 {
        self.missing_total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CSV parsing ─────────────────────────────────────────────────────

    #[test]
    fn csv_row_parses_quoted_fields_with_commas_and_escapes() {
        let row = parse_csv_row(r#""DESK","Mi,Control","say ""hi"" now",3"#);
        assert_eq!(row, vec!["DESK", "Mi,Control", "say \"hi\" now", "3"]);
    }

    #[test]
    fn csv_row_handles_empty_and_unterminated_quotes() {
        assert_eq!(parse_csv_row("a,,c"), vec!["a", "", "c"]);
        // Lenient: text after a closing quote continues the same field
        // (Excel-style), which never loses scheduler columns.
        assert_eq!(parse_csv_row(r#""open,"x"#), vec!["open,x"]);
    }

    #[test]
    fn query_csv_matches_fields_by_header_name() {
        let csv = "\"HostName\",\"TaskName\",\"Next Run Time\",\"Status\",\"Logon Mode\",\"Last Run Time\",\"Last Result\",\"Author\"\r\n\
                   \"DESK\",\"MiControlElevated\",\"N/A\",\"Ready\",\"Interactive only\",\"2026-11-20 10:00:00\",\"267011\",\"DESK\\mafsc\"\r\n";
        let st = parse_query_csv(csv).expect("parsable");
        assert_eq!(st.status, "Ready");
        assert_eq!(st.last_result, 267011);
        assert_eq!(st.last_run, "2026-11-20 10:00:00");
    }

    #[test]
    fn query_csv_handles_reordered_columns_and_bad_numbers() {
        let csv = "\"Status\",\"Last Result\",\"Last Run Time\"\r\n\"Running\",\"x\",\"\"\r\n";
        let st = parse_query_csv(csv).expect("parsable");
        assert_eq!(st.status, "Running");
        assert_eq!(st.last_result, 0, "non-numeric result falls back to 0");
        assert_eq!(st.last_run, "");
    }

    #[test]
    fn query_csv_rejects_header_only_and_garbage() {
        assert!(parse_query_csv("\"Status\",\"Last Result\"\r\n").is_none());
        assert!(parse_query_csv("").is_none());
    }

    // ── Error classification ────────────────────────────────────────────

    #[test]
    fn classify_output_maps_not_found_in_english_and_portuguese() {
        assert_eq!(
            classify_output("ERROR: The system cannot find the file specified."),
            FailKind::NotFound
        );
        assert_eq!(
            classify_output("ERRO: A tarefa não existe."),
            FailKind::NotFound
        );
        assert_eq!(
            classify_output("INFO: There are no scheduled tasks."),
            FailKind::NotFound
        );
    }

    #[test]
    fn classify_output_maps_access_denied_and_other() {
        assert_eq!(
            classify_output("ERROR: Access is denied."),
            FailKind::AccessDenied
        );
        assert_eq!(
            classify_output("ERRO: Acesso negado."),
            FailKind::AccessDenied
        );
        assert_eq!(classify_output("ERROR: boom"), FailKind::Other);
    }

    #[test]
    fn not_found_errors_carry_the_stable_prefix() {
        let err = not_found_err("run", "MiControlElevated", "boom");
        assert!(is_not_found(&err));
        let other = AppError::new(ErrorCode::TaskControlFailed, "run(x): boom");
        assert!(!is_not_found(&other));
    }

    // ── XML helpers ─────────────────────────────────────────────────────

    #[test]
    fn elevated_task_xml_is_byte_stable_and_carries_the_arguments() {
        let xml = elevated_task_xml(r"C:\Program Files\MiControl\micontrol.exe");
        assert!(xml.contains(r#"<Command>"C:\Program Files\MiControl\micontrol.exe"</Command>"#));
        assert!(xml.contains("<Arguments>--elevated</Arguments>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT120S</ExecutionTimeLimit>"));
    }

    #[test]
    fn xml_targets_path_accepts_all_storage_forms() {
        let exe = r"C:\Program Files\MiControl\micontrol.exe";
        let xml = elevated_task_xml(exe);
        assert!(xml_targets_path(&xml, exe));
        // Forward-slash variant (some registrations store this form).
        let fwd = xml.replace('\\', "/");
        assert!(xml_targets_path(&fwd, exe));
        // A different exe never matches.
        assert!(!xml_targets_path(&xml, r"C:\other\tool.exe"));
        assert!(!xml_targets_path(&xml, ""));
    }

    // ── L-01 gate: one create per backoff window, one WARN per episode ──

    #[test]
    fn missing_task_emits_one_warn_per_episode_with_accumulated_count() {
        let mut g = TaskGate::default();
        g.set_exists(0, false);
        let first = g.decide(0);
        assert_eq!(
            first,
            TaskDecision::Missing {
                emit_warn: true,
                missing_total: 1
            }
        );
        // Every following observation counts but never re-warns.
        for n in 1..=10u64 {
            assert_eq!(
                g.decide(n),
                TaskDecision::Missing {
                    emit_warn: false,
                    missing_total: n + 1
                }
            );
        }
        assert_eq!(g.missing_total(), 11);
        // Task reappears: episode closes; a NEW missing task opens a NEW
        // episode with a fresh WARN.
        g.set_exists(20, true);
        assert_eq!(g.decide(20), TaskDecision::Attempt);
        g.set_exists(21, false);
        assert_eq!(
            g.decide(21),
            TaskDecision::Missing {
                emit_warn: true,
                missing_total: 12
            }
        );
    }

    #[test]
    fn heal_attempts_back_off_exponentially_and_reset_on_success() {
        let mut g = TaskGate::default();
        g.set_exists(0, false);
        assert!(g.heal_attempt_allowed(0));
        g.note_heal_failed(0);
        assert!(
            !g.heal_attempt_allowed(59_999),
            "blocked inside 1 min window"
        );
        assert!(g.heal_attempt_allowed(60_000));
        g.note_heal_failed(60_000);
        assert!(
            !g.heal_attempt_allowed(60_000 + 299_999),
            "blocked inside 5 min window"
        );
        assert!(g.heal_attempt_allowed(60_000 + 300_000));
        g.note_heal_failed(360_000);
        assert!(
            !g.heal_attempt_allowed(360_000 + 1_799_999),
            "blocked inside 30 min window"
        );
        assert!(g.heal_attempt_allowed(360_000 + 1_800_000));
        // The backoff CAPS at 30 min: further failures never grow past it.
        g.note_heal_failed(360_000 + 1_800_000);
        assert!(g.heal_attempt_allowed(360_000 + 1_800_000 + 1_800_000));
        // Success resets the schedule.
        g.note_healed(1_000_000);
        assert!(g.heal_attempt_allowed(1_000_000));
        assert_eq!(
            g.decide(1_000_001),
            TaskDecision::Attempt,
            "healed task exists again"
        );
    }

    #[test]
    fn exists_cache_respects_the_refresh_window() {
        let mut g = TaskGate::default();
        assert!(g.exists_refresh_due(0), "first consult always due");
        g.set_exists(0, true);
        assert!(!g.exists_refresh_due(59_999));
        assert!(g.exists_refresh_due(60_000));
    }
}
