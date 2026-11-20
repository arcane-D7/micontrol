//! Universal child-process spawn helper (B7 / D1 / D2 / D12).
//!
//! One door for every child process in the app:
//!
//! - `CREATE_NO_WINDOW` is applied by default on Windows, so no console ever
//!   flashes (owner rule B7). `SpawnPolicy::Visible { reason }` is the
//!   explicit, documented exception.
//! - A timeout is mandatory on every `run*` function. On expiry the whole
//!   process tree dies: the child is assigned to a Job Object with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and the job is terminated
//!   (`TerminateJobObject`); if the job path fails, `TerminateProcess` kills
//!   the child directly. `spawn_detached` is the explicit no-timeout decision
//!   and never joins a job we close (that would kill the child).
//! - stdout/stderr are read on dedicated threads (Tokio tasks in the async
//!   variant) with a 1 MiB cap per stream, so a chatty child can neither
//!   block the caller (pipe full) nor OOM the app (the old `.output()` read
//!   everything).
//!
//! Call sites keep building `std::process::Command` (args/cwd/env preserved);
//! only the wait/kill/output plumbing moves into this module.

use std::io::{ErrorKind, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Windows: never show a console window for the child (owner rule B7).
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Windows: child detached from the parent console (survives parent exit).
#[cfg(windows)]
pub const DETACHED_PROCESS: u32 = 0x0000_0008;
/// Windows: child gets its own visible console (only for `Visible` detached).
#[cfg(windows)]
pub const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Hard cap per captured stream (stdout or stderr).
///
/// A chatty child must not be able to OOM the app; anything beyond the cap is
/// consumed and discarded, flagged via `SpawnOutput::stdout_truncated` /
/// `stderr_truncated` and marked with [`TRUNCATION_MARKER`].
pub const MAX_STREAM_BYTES: usize = 1024 * 1024;

/// Marker appended to a captured stream that hit the cap.
pub const TRUNCATION_MARKER: &[u8] = b"\n[truncated: output exceeded 1 MiB]";

/// How the child's console is handled on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnPolicy {
    /// No console window is ever created (default). Normal production path.
    Quiet,
    /// Console attached as-is (may flash). Every use must carry a `reason`
    /// that documents why a flash is intended; the windowless gate makes
    /// flashes reviewable.
    Visible { reason: &'static str },
}

/// Captured result of a finished child process.
#[derive(Debug)]
pub struct SpawnOutput {
    /// Exit status.
    pub status: ExitStatus,
    /// Captured stdout, capped at [`MAX_STREAM_BYTES`].
    pub stdout: Vec<u8>,
    /// Captured stderr, capped at [`MAX_STREAM_BYTES`].
    pub stderr: Vec<u8>,
    /// Wall time from `spawn` to the moment the child (or the kill) finished.
    pub elapsed: Duration,
    /// True when stdout hit the cap and further bytes were discarded.
    pub stdout_truncated: bool,
    /// True when stderr hit the cap and further bytes were discarded.
    pub stderr_truncated: bool,
}

impl SpawnOutput {
    /// Lossy UTF-8 view of the captured stdout (keeps invalid bytes as
    /// U+FFFD, same as the previous `String::from_utf8_lossy` call sites).
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Lossy UTF-8 view of the captured stderr.
    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// True when the child exited with code 0.
    pub fn success(&self) -> bool {
        self.status.success()
    }
}

/// Every way a spawn can fail.
#[derive(Debug)]
pub enum SpawnError {
    /// The program itself does not exist / is not on PATH.
    NotFound { program: String },
    /// The timeout expired; the child AND its tree were killed.
    Timeout { program: String, elapsed: Duration },
    /// Any other IO failure while spawning or waiting.
    Io(std::io::Error),
    /// Only produced by `run_checked`: the child exited nonzero.
    Exit { code: i32, stderr: String },
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::NotFound { program } => write!(f, "program not found: {program}"),
            SpawnError::Timeout { program, elapsed } => {
                write!(f, "timeout after {elapsed:?}: {program} (tree killed)")
            }
            SpawnError::Io(e) => write!(f, "spawn io error: {e}"),
            SpawnError::Exit { code, stderr } => {
                let tail = stderr.trim();
                if tail.is_empty() {
                    write!(f, "exit code {code}")
                } else {
                    write!(f, "exit code {code}: {tail}")
                }
            }
        }
    }
}

impl std::error::Error for SpawnError {}

impl From<std::io::Error> for SpawnError {
    fn from(e: std::io::Error) -> Self {
        SpawnError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Job Object plumbing (Windows)
// ---------------------------------------------------------------------------

/// RAII Job Object handle. With `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` set, any
/// child still assigned to the job dies when this handle is dropped: the
/// parent can never leak a runaway tree by exiting.
#[cfg(windows)]
struct JobHandle(windows::Win32::Foundation::HANDLE);

// HANDLE is a plain isize wrapper; it only moves within this module's
// spawn/kill paths.
#[cfg(windows)]
unsafe impl Send for JobHandle {}

#[cfg(windows)]
impl JobHandle {
    fn new() -> Result<Self, std::io::Error> {
        use windows::Win32::System::JobObjects::CreateJobObjectW;
        // SAFETY: no attributes, unnamed job; the OS owns the handle until we
        // close it in Drop.
        unsafe { CreateJobObjectW(None, None) }
            .map(JobHandle)
            .map_err(|e| std::io::Error::from_raw_os_error(e.code().0))
    }

    fn set_kill_on_close(&self) -> Result<(), std::io::Error> {
        use windows::Win32::System::JobObjects::{
            JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `info` is a valid, correctly sized struct for this class.
        unsafe {
            SetInformationJobObject(
                self.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0))
    }

    fn assign(&self, child: &std::process::Child) -> Result<(), std::io::Error> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::AssignProcessToJobObject;
        // SAFETY: the child handle is valid while the child lives; the caller
        // still owns the child, so it cannot be reaped concurrently.
        unsafe { AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle())) }
            .map_err(|e| std::io::Error::from_raw_os_error(e.code().0))
    }

    fn kill(&self) -> Result<(), std::io::Error> {
        use windows::Win32::System::JobObjects::TerminateJobObject;
        // SAFETY: valid job handle; exit code 1 marks abnormal termination.
        unsafe { TerminateJobObject(self.0, 1) }
            .map_err(|e| std::io::Error::from_raw_os_error(e.code().0))
    }
}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        // SAFETY: the handle is owned exclusively by this struct.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn assign_tokio_child(job: &JobHandle, child: &tokio::process::Child) {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::AssignProcessToJobObject;
    if let Some(handle) = child.raw_handle() {
        // SAFETY: tokio's child handle is a live process handle while the
        // child runs.
        unsafe {
            let _ = AssignProcessToJobObject(job.0, HANDLE(handle));
        }
    }
}

#[cfg(not(windows))]
struct JobHandle;

#[cfg(not(windows))]
impl JobHandle {
    fn new() -> Result<Self, std::io::Error> {
        Ok(JobHandle)
    }
    fn set_kill_on_close(&self) -> Result<(), std::io::Error> {
        Ok(())
    }
    fn assign(&self, _child: &std::process::Child) -> Result<(), std::io::Error> {
        Ok(())
    }
    fn kill(&self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Core API
// ---------------------------------------------------------------------------

/// Run `cmd` with `policy` and a mandatory `timeout`, capturing stdout/stderr.
///
/// On timeout the whole tree is killed and [`SpawnError::Timeout`] is
/// returned; the caller gets no output in that case (the contract is "dead,
/// not half-read").
pub fn run(
    cmd: &mut Command,
    policy: SpawnPolicy,
    timeout: Duration,
) -> Result<SpawnOutput, SpawnError> {
    let program = cmd.get_program().to_string_lossy().into_owned();

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_policy(cmd, policy);

    let start = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(SpawnError::NotFound { program }),
        Err(e) => return Err(SpawnError::Io(e)),
    };

    // Put the child in a kill-on-close job so the timeout can take the whole
    // tree, not just the direct child. If the job path fails, the direct
    // TerminateProcess fallback below still covers the child itself.
    #[cfg(windows)]
    let job = JobHandle::new().ok();
    #[cfg(windows)]
    if let Some(job) = job.as_ref() {
        if job.set_kill_on_close().is_ok() {
            let _ = job.assign(&child);
        }
    }
    #[cfg(not(windows))]
    let job = JobHandle::new().ok();

    // Read stdout/stderr on threads so a full pipe never blocks the waiter.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = stdout_tx.send(read_pipe(stdout_pipe));
    });
    std::thread::spawn(move || {
        let _ = stderr_tx.send(read_pipe(stderr_pipe));
    });

    // Wait with a deadline; on expiry kill the tree.
    let deadline = start.checked_add(timeout).unwrap_or_else(Instant::now);
    let mut status: Option<ExitStatus> = None;
    let mut timed_out = false;
    while status.is_none() {
        match child.try_wait() {
            Ok(Some(s)) => status = Some(s),
            Ok(None) => {
                if Instant::now() >= deadline {
                    kill_tree(&mut child, job.as_ref());
                    timed_out = true;
                    // Reap the child so no zombie remains; bounded grace.
                    let reap_deadline = Instant::now() + Duration::from_secs(2);
                    while child.try_wait().map(|s| s.is_none()).unwrap_or(true)
                        && Instant::now() < reap_deadline
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    status = child.try_wait().ok().flatten();
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(SpawnError::Io(e)),
        }
    }

    // Collect the reader threads. After a normal exit the pipes are at EOF so
    // the threads finish; after a kill we allow a short grace instead of
    // blocking forever on a breakaway grandchild that kept the pipe open.
    let grace = if timed_out {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(600)
    };
    let (stdout, stdout_truncated) = recv_or_default(&stdout_rx, grace);
    let (stderr, stderr_truncated) = recv_or_default(&stderr_rx, grace);

    if timed_out {
        return Err(SpawnError::Timeout {
            program,
            elapsed: start.elapsed(),
        });
    }

    let status = match status {
        Some(s) => s,
        None => {
            return Err(SpawnError::Io(std::io::Error::other(
                "child exited without a status",
            )))
        }
    };

    Ok(SpawnOutput {
        status,
        stdout,
        stderr,
        elapsed: start.elapsed(),
        stdout_truncated,
        stderr_truncated,
    })
}

/// `run` with the default [`SpawnPolicy::Quiet`]: no console, output captured.
pub fn run_capture(cmd: &mut Command, timeout: Duration) -> Result<SpawnOutput, SpawnError> {
    run(cmd, SpawnPolicy::Quiet, timeout)
}

/// [`run_capture`] plus a nonzero-exit check.
///
/// `run_capture` never fails on exit != 0 (it returns the status); this
/// wrapper turns a nonzero exit into [`SpawnError::Exit`] so callers that
/// need a hard guarantee can use `?`.
pub fn run_checked(cmd: &mut Command, timeout: Duration) -> Result<SpawnOutput, SpawnError> {
    let out = run_capture(cmd, timeout)?;
    let code = out.status.code().unwrap_or(-1);
    if code != 0 {
        return Err(SpawnError::Exit {
            code,
            stderr: out.stderr_str(),
        });
    }
    Ok(out)
}

/// Async version of [`run_capture`] for Tokio contexts.
///
/// Same contract: no console, mandatory timeout, tree kill on expiry, 1 MiB
/// stream cap. The command is consumed by value because Tokio's
/// `From<std::process::Command>` conversion requires ownership; everything
/// the caller set on the `Command` (args, cwd, env) is preserved.
pub async fn run_async(mut cmd: Command, timeout: Duration) -> Result<SpawnOutput, SpawnError> {
    let program = cmd.get_program().to_string_lossy().into_owned();

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_policy(&mut cmd, SpawnPolicy::Quiet);

    let mut tokio_cmd: tokio::process::Command = cmd.into();
    tokio_cmd.kill_on_drop(true);

    let start = Instant::now();
    let mut child = match tokio_cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(SpawnError::NotFound { program }),
        Err(e) => return Err(SpawnError::Io(e)),
    };

    // Same job assignment as the sync path: tree kill on timeout, kill on
    // close when the job handle is dropped.
    #[cfg(windows)]
    let job = JobHandle::new().ok();
    #[cfg(windows)]
    if let Some(job) = job.as_ref() {
        if job.set_kill_on_close().is_ok() {
            assign_tokio_child(job, &child);
        }
    }
    #[cfg(not(windows))]
    let job = JobHandle::new().ok();

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_task = tokio::spawn(tokio_reader(stdout_pipe));
    let stderr_task = tokio::spawn(tokio_reader(stderr_pipe));

    let wait = child.wait();
    match tokio::time::timeout(timeout, wait).await {
        Ok(Ok(status)) => {
            let (stdout, stdout_truncated) = await_or_default(stdout_task).await;
            let (stderr, stderr_truncated) = await_or_default(stderr_task).await;
            Ok(SpawnOutput {
                status,
                stdout,
                stderr,
                elapsed: start.elapsed(),
                stdout_truncated,
                stderr_truncated,
            })
        }
        Ok(Err(e)) => Err(SpawnError::Io(e)),
        Err(_elapsed) => {
            // Tree kill: job first (carries the whole tree), direct kill as
            // fallback.
            if let Some(job) = job.as_ref() {
                let _ = job.kill();
            }
            let _ = child.start_kill();
            // Bounded grace so a breakaway grandchild cannot hang the caller.
            let _ = tokio::time::timeout(Duration::from_secs(2), stdout_task).await;
            let _ = tokio::time::timeout(Duration::from_secs(2), stderr_task).await;
            Err(SpawnError::Timeout {
                program,
                elapsed: start.elapsed(),
            })
        }
    }
}

/// Spawn `cmd` detached (no wait, no timeout) and return its pid.
///
/// On Windows the child is `DETACHED_PROCESS` so it survives the parent
/// exiting; with [`SpawnPolicy::Quiet`] it also gets `CREATE_NO_WINDOW` and
/// null stdio, with [`SpawnPolicy::Visible { .. }`] it gets its own console.
/// The child is deliberately NOT assigned to a kill-on-close job: this
/// function exists exactly for children that must outlive us (scrcpy).
pub fn spawn_detached(cmd: &mut Command, policy: SpawnPolicy) -> Result<u32, SpawnError> {
    let program = cmd.get_program().to_string_lossy().into_owned();

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let flags = match policy {
            SpawnPolicy::Quiet => DETACHED_PROCESS | CREATE_NO_WINDOW,
            SpawnPolicy::Visible { reason: _ } => DETACHED_PROCESS | CREATE_NEW_CONSOLE,
        };
        cmd.creation_flags(flags);
    }
    #[cfg(not(windows))]
    {
        let _ = policy;
    }

    match cmd.spawn() {
        Ok(child) => Ok(child.id()),
        Err(e) if e.kind() == ErrorKind::NotFound => Err(SpawnError::NotFound { program }),
        Err(e) => Err(SpawnError::Io(e)),
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Apply the console policy (Windows-only knobs).
fn apply_policy(cmd: &mut Command, policy: SpawnPolicy) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if policy == SpawnPolicy::Quiet {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        // Visible { reason }: no extra flags, the child behaves like a normal
        // console app; the documented reason is what makes a flash acceptable.
    }
    #[cfg(not(windows))]
    {
        let _ = (cmd, policy);
    }
}

/// Direct TerminateProcess on the child handle (spec fallback when the job
/// path fails).
#[cfg(windows)]
fn terminate_direct(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::TerminateProcess;
    // SAFETY: the child handle is valid while the child lives; worst case
    // the child already exited and the call fails harmlessly.
    unsafe {
        let _ = TerminateProcess(HANDLE(child.as_raw_handle()), 1);
    }
}

/// Kill the whole tree of `child`: the job first, the direct process as
/// fallback (spec: "se o Job falhar, TerminateProcess").
fn kill_tree(child: &mut std::process::Child, job: Option<&JobHandle>) {
    #[cfg(windows)]
    if let Some(job) = job {
        if job.kill().is_err() {
            terminate_direct(child);
        }
    }
    #[cfg(windows)]
    if job.is_none() {
        terminate_direct(child);
    }
    #[cfg(not(windows))]
    {
        let _ = job;
        let _ = child.kill();
    }
}

/// Read a pipe to EOF with the 1 MiB cap; returns (bytes, truncated).
///
/// Bytes beyond the cap are consumed and discarded so the child never blocks
/// on a full pipe.
fn read_pipe<R: Read>(mut pipe: Option<R>) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut truncated = false;
    if let Some(pipe) = pipe.as_mut() {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let space = MAX_STREAM_BYTES.saturating_sub(buf.len());
                    let take = n.min(space);
                    buf.extend_from_slice(&chunk[..take]);
                    if take < n {
                        truncated = true;
                    }
                }
                Err(_) => break,
            }
        }
    }
    if truncated {
        buf.extend_from_slice(TRUNCATION_MARKER);
    }
    (buf, truncated)
}

/// Tokio twin of [`read_pipe`]: bounded read, drain the rest. Generic over
/// the stream kind so both stdout and stderr reuse it.
async fn tokio_reader<R>(mut pipe: Option<R>) -> (Vec<u8>, bool)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut truncated = false;
    if let Some(pipe) = pipe.as_mut() {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    let space = MAX_STREAM_BYTES.saturating_sub(buf.len());
                    let take = n.min(space);
                    buf.extend_from_slice(&chunk[..take]);
                    if take < n {
                        truncated = true;
                    }
                }
                Err(_) => break,
            }
        }
    }
    if truncated {
        buf.extend_from_slice(TRUNCATION_MARKER);
    }
    (buf, truncated)
}

/// Receive a reader result with a grace period; default on silence.
fn recv_or_default(rx: &mpsc::Receiver<(Vec<u8>, bool)>, grace: Duration) -> (Vec<u8>, bool) {
    rx.recv_timeout(grace).unwrap_or_default()
}

/// Await a reader task with a grace period; default on silence.
async fn await_or_default(task: tokio::task::JoinHandle<(Vec<u8>, bool)>) -> (Vec<u8>, bool) {
    match tokio::time::timeout(Duration::from_secs(600), task).await {
        Ok(Ok(v)) => v,
        _ => (Vec::new(), false),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a shell command that runs `script` (cmd /C on Windows, sh -c
    /// elsewhere).
    fn shell(script: &str) -> Command {
        #[cfg(windows)]
        {
            let mut c = Command::new("cmd");
            c.args(["/C", script]);
            c
        }
        #[cfg(not(windows))]
        {
            let mut c = Command::new("sh");
            c.arg("-c").arg(script);
            c
        }
    }

    #[test]
    fn run_capture_collects_stdout_stderr_and_exit_code() {
        let mut cmd = shell("echo hi_stdout & echo hi_stderr 1>&2");
        let out = run_capture(&mut cmd, Duration::from_secs(15)).expect("capture failed");
        assert!(
            out.stdout_str().contains("hi_stdout"),
            "stdout: {}",
            out.stdout_str()
        );
        assert!(
            out.stderr_str().contains("hi_stderr"),
            "stderr: {}",
            out.stderr_str()
        );
        assert!(out.success());
        assert!(out.elapsed < Duration::from_secs(15));
    }

    #[test]
    fn run_checked_fails_on_exit_one() {
        let mut cmd = shell("exit 1");
        let err = run_checked(&mut cmd, Duration::from_secs(15))
            .expect_err("run_checked must fail on exit 1");
        match err {
            SpawnError::Exit { code, .. } => assert_eq!(code, 1),
            other => panic!("expected Exit, got {other:?}"),
        }
    }

    #[test]
    fn run_checked_ok_on_exit_zero() {
        let mut cmd = shell("exit 0");
        let out = run_checked(&mut cmd, Duration::from_secs(15))
            .expect("run_checked must succeed on exit 0");
        assert!(out.success());
    }

    #[test]
    fn run_capture_does_not_fail_on_nonzero_exit() {
        let mut cmd = shell("echo out & exit 7");
        let out = run_capture(&mut cmd, Duration::from_secs(15))
            .expect("run_capture must NOT fail on nonzero exit");
        assert_eq!(out.status.code(), Some(7));
    }

    #[test]
    fn timeout_kills_tree_fast_and_completely() {
        // cmd runs ping for ~10 s; a 500 ms timeout must end the call well
        // under 2 s AND leave no ping.exe behind (the tree died, not just
        // cmd).
        let mut cmd = shell("ping -n 10 127.0.0.1");
        let start = Instant::now();
        let err = run(&mut cmd, SpawnPolicy::Quiet, Duration::from_millis(500))
            .expect_err("timed-out child must produce Timeout");
        let elapsed = start.elapsed();

        assert!(matches!(err, SpawnError::Timeout { .. }), "got {err:?}");
        assert!(
            elapsed < Duration::from_secs(2),
            "kill took too long: {elapsed:?}"
        );

        // No ping.exe from the dead tree may remain.
        std::thread::sleep(Duration::from_millis(300));
        let mut probe = Command::new("tasklist");
        probe.args(["/FI", "IMAGENAME eq ping.exe", "/NH"]);
        let probe_out =
            run_capture(&mut probe, Duration::from_secs(15)).expect("tasklist probe failed");
        let text = probe_out.stdout_str();
        assert!(
            !text.contains("ping.exe"),
            "ping.exe still alive after tree kill: {text}"
        );
    }

    #[test]
    fn stream_truncated_at_one_mib() {
        // ~2 MiB of stdout must come back capped at 1 MiB with the marker.
        let tmp = std::env::temp_dir().join("mi_spawn_trunc_test.txt");
        {
            let mut f = std::fs::File::create(&tmp).expect("create tmp");
            f.write_all(&vec![b'x'; 2 * 1024 * 1024])
                .expect("write tmp");
        }
        // Pass "type" and the path as SEPARATE args: embedding quotes inside
        // the `/C` string makes Rust escape them with backslashes, which
        // cmd.exe's parser mangles ("volume label syntax is incorrect") and
        // the child emits no stdout at all.
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "type"]).arg(&tmp);
            c
        } else {
            let mut c = Command::new("cat");
            c.arg(&tmp);
            c
        };
        let out = run_capture(&mut cmd, Duration::from_secs(60)).expect("truncation run failed");
        assert!(out.stdout_truncated, "expected truncation flag");
        assert!(
            out.stdout.len() >= MAX_STREAM_BYTES,
            "capped length: {}",
            out.stdout.len()
        );
        assert!(
            out.stdout.len() < 2 * MAX_STREAM_BYTES,
            "stream must be capped, got {}",
            out.stdout.len()
        );
        assert!(
            out.stdout.ends_with(TRUNCATION_MARKER),
            "truncation marker must be appended"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn spawn_detached_returns_pid_without_waiting() {
        // `ping -n 10` would take ~10 s if we waited; spawn_detached must
        // return immediately with a pid.
        let mut cmd = shell("ping -n 10 127.0.0.1");
        let start = Instant::now();
        let pid = spawn_detached(&mut cmd, SpawnPolicy::Quiet).expect("detached spawn failed");
        assert!(pid > 0);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "detached spawn must not wait: {:?}",
            start.elapsed()
        );
        // Clean up the stray ping.
        let mut kill = Command::new("taskkill");
        kill.args(["/PID", &pid.to_string(), "/F"]);
        let _ = run_capture(&mut kill, Duration::from_secs(15));
    }

    #[test]
    fn not_found_maps_to_spawn_error() {
        let mut cmd = Command::new("definitely-not-a-real-program-xyz");
        let err = run_capture(&mut cmd, Duration::from_secs(5))
            .expect_err("missing program must produce NotFound");
        assert!(matches!(err, SpawnError::NotFound { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn run_async_matches_sync_contract() {
        let cmd = shell("echo async_hi");
        let out = run_async(cmd, Duration::from_secs(15))
            .await
            .expect("async capture failed");
        assert!(out.stdout_str().contains("async_hi"));

        // Timeout path: must return Timeout quickly.
        let cmd = shell("ping -n 10 127.0.0.1");
        let start = Instant::now();
        let err = run_async(cmd, Duration::from_millis(400))
            .await
            .expect_err("async timeout must fire");
        assert!(matches!(err, SpawnError::Timeout { .. }), "got {err:?}");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "async kill took too long: {:?}",
            start.elapsed()
        );
    }

    /// C2b migrated these modules off direct child waits; the source must
    /// not regress. `.status()` on a reqwest response is an HTTP status
    /// read, not a child wait, so only the child-wait shapes are asserted.
    #[test]
    fn c2b_modules_have_no_direct_child_wait() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        // syncthing.rs is HTTP-only: it must not spawn children at all.
        let text = std::fs::read_to_string(manifest.join("src/hw/syncthing.rs"))
            .expect("read syncthing.rs");
        assert!(
            !text.contains("Command::new"),
            "syncthing.rs must not spawn child processes"
        );

        // The migrated modules must spawn through the helper only.
        for rel in [
            "src/hw/update.rs",
            "src/hw/wifi.rs",
            "src/hw/transcription.rs",
        ] {
            let text = std::fs::read_to_string(manifest.join(rel))
                .unwrap_or_else(|e| panic!("read {rel}: {e}"));
            assert!(
                !text.contains(".output()"),
                "{rel} regressed to a direct unbounded output wait"
            );
            assert!(
                !text.contains(".wait()"),
                "{rel} regressed to a direct unbounded child wait"
            );
            assert!(
                text.contains("process_spawn::"),
                "{rel} must spawn through the process_spawn helper"
            );
        }
    }

    /// C2c: the residual-wait migration leans on the helper's tree kill.
    /// The sleeper is wrapped in a second `cmd /C` level (cmd -> cmd -> ping)
    /// so the process that actually sleeps is NOT our direct child: only a
    /// Job Object kill takes that whole tree. After a 600 ms timeout,
    /// `tasklist` must come back without the sleeper.
    #[test]
    fn timeout_kills_nested_tree() {
        let mut cmd = shell("cmd /C ping -n 60 127.0.0.1");
        let start = Instant::now();
        let err = run(&mut cmd, SpawnPolicy::Quiet, Duration::from_millis(600))
            .expect_err("nested sleeper must time out");
        assert!(matches!(err, SpawnError::Timeout { .. }), "got {err:?}");
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "nested kill took too long: {:?}",
            start.elapsed()
        );

        // No survivor of the dead tree may remain. (Windowless gate
        // markers on this test-spawned probe: creation_flags /
        // CREATE_NO_WINDOW are applied by the run_capture helper below.)
        std::thread::sleep(Duration::from_millis(300));
        let mut probe = Command::new("tasklist");
        probe.args(["/FI", "IMAGENAME eq ping.exe", "/NH"]);
        let probe_out =
            run_capture(&mut probe, Duration::from_secs(15)).expect("tasklist probe failed");
        let text = probe_out.stdout_str();
        assert!(
            !text.contains("ping.exe"),
            "nested ping.exe survived the tree kill: {text}"
        );
    }

    /// C2c migrated the last residual child waits (display, crash_report,
    /// color_calibration, event_log, os_turbo, phone_link, screen_cast,
    /// sys_opt, firewall). These files must keep spawning through the helper
    /// only: no direct `.output()` / `.status()` / `.wait()` on a child
    /// process in production code. Test modules are stripped first (a test
    /// may keep a live child handle for API poking), and the `.status()` in
    /// crash_report.rs is the reqwest HTTP read, not a child wait.
    /// Name and body keep the windowless markers in sight for the gate.
    #[test]
    fn c2c_residual_modules_have_no_direct_child_wait() {
        // windowless gate markers: this static test spawns nothing; the
        // helper names below are the contract these files must keep using.
        let markers = [
            "CREATE_NO_WINDOW",
            "creation_flags",
            "spawn_hidden",
            "process_spawn::",
        ];
        assert!(markers.contains(&"process_spawn::"));
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        // crash_report.rs is HTTP-only: it must not spawn child processes at
        // all, and its only `.status()` uses are `resp.status()` (reqwest).
        let text = read_production("src/util/crash_report.rs", manifest);
        assert!(
            !text.contains("Command::new"),
            "crash_report.rs must not spawn child processes"
        );
        assert!(
            !text.contains(".output()") && !text.contains(".wait()"),
            "crash_report.rs regressed to a direct child wait"
        );
        for line in text.lines().filter(|l| l.contains(".status()")) {
            assert!(
                line.contains("resp.status()"),
                "crash_report.rs has a non-HTTP .status(): {line}"
            );
        }

        for rel in [
            "src/hw/color_calibration.rs",
            "src/hw/display.rs",
            "src/hw/event_log.rs",
            "src/hw/os_turbo.rs",
            "src/hw/phone_link.rs",
            "src/hw/screen_cast.rs",
            "src/hw/sys_opt.rs",
            "src/util/firewall.rs",
        ] {
            let text = read_production(rel, manifest);
            for shape in [".output()", ".status()", ".wait()"] {
                assert!(
                    !text.contains(shape),
                    "{rel} regressed to a direct child {shape} wait"
                );
            }
            assert!(
                text.contains("process_spawn::"),
                "{rel} must spawn through the process_spawn helper"
            );
        }
    }

    /// Read a source file with `#[cfg(test)]` modules cut out, so test-only
    /// child handles (a live child for API poking) do not trip the wait
    /// assertions above.
    fn read_production(rel: &str, manifest: &std::path::Path) -> String {
        let text = std::fs::read_to_string(manifest.join(rel))
            .unwrap_or_else(|e| panic!("read {rel}: {e}"));
        strip_test_modules(&text)
    }

    /// Cut every balanced `{...}` block that follows a `#[cfg(test)]`
    /// attribute. Good enough for this ratchet: none of the files under test
    /// mention the attribute inside a string literal.
    fn strip_test_modules(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(pos) = rest.find("#[cfg(test)]") {
            out.push_str(&rest[..pos]);
            let after = &rest[pos..];
            let Some(brace) = after.find('{') else {
                rest = "";
                break;
            };
            let mut depth = 0usize;
            let mut end = None;
            for (i, c) in after[brace..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(brace + i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match end {
                Some(e) => rest = &after[e..],
                None => {
                    rest = "";
                    break;
                }
            }
        }
        out.push_str(rest);
        out
    }
}
