//! Payload application: the full set of install points, ported 1:1 from the
//! legacy NSIS hooks (`installer-hooks.nsi`) so the decoupled shell produces
//! exactly the same machine state the old installer did.
//!
//! Install points (in order):
//!   1. Stop app + auxiliary services (file-lock release, pre-extract)
//!   2. WebView2 runtime presence (first install only)
//!   3. Extract payload (never touches `setup\`)
//!   4. Run-key autostart
//!   5. Hardware drivers (pnputil: VirtualControlHID + IoTDriver)
//!   6. DriverStore ecram_service deploy + IoTSvc recreate
//!   7. MiControlElevated scheduled task
//!   8. MiControlBridge service (`micontrol_bridge.exe install` + fallback)
//!   9. Face Unlock service + Credential Provider registry
//!  10. Post-install verification
//!  11. Uninstaller registration + shortcuts (first install)
//!  12. Launch the app for the logged-on user

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ctx;
use crate::payload;
use crate::sys;
use crate::Options;

// ── S65-005: transactional apply ──────────────────────────────────────────────
//
// The apply runs as a transaction:
//   1. extract the payload into a same-volume staging dir (never the live dir)
//   2. stop the app/services, then back up the previously installed critical
//      files into a hidden backup dir
//   3. swap the staged files into place and run every install point
//   4. verify the result — if ANY required step failed, restore the backup
//      (binaries + critical install points) and report original + rollback
//      outcomes
//   5. only after verification passes are the backup + staging dirs deleted
//
// The permanent setup shell (`<dir>\setup\`) is never staged, backed up,
// swapped or restored. A crash between backup and commit is repaired on the
// next launch by `recover_interrupted` (restores the leftover backup and
// cleans up abandoned staging directories), making repeated apply/recovery
// operations idempotent.

#[cfg(test)]
use std::sync::Mutex;

/// Test hook: when true, every step that touches the REAL system (processes,
/// services, drivers, registry, shortcuts) short-circuits to Ok.
#[cfg(test)]
static SYSTEM_NOOP: Mutex<bool> = Mutex::new(false);
/// Test hook: names of steps that must fail on the next apply. See `fail_check`.
#[cfg(test)]
static FAIL_POINTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[cfg(test)]
pub fn set_system_noop(v: bool) {
    *SYSTEM_NOOP.lock().unwrap() = v;
}
#[cfg(test)]
pub fn inject_failure(point: &str) {
    FAIL_POINTS.lock().unwrap().push(point.to_string());
}
#[cfg(test)]
pub fn clear_failures() {
    FAIL_POINTS.lock().unwrap().clear();
}

/// Injected-failure hook — a no-op outside tests.
fn fail_check(_point: &str) -> Result<(), String> {
    #[cfg(test)]
    if FAIL_POINTS.lock().unwrap().iter().any(|p| p == _point) {
        return Err(format!("injected failure at step '{_point}'"));
    }
    Ok(())
}

/// Outcome collector for the transaction steps.
pub struct StepReport {
    /// (step name, error) for every REQUIRED step that failed.
    pub failures: Vec<(String, String)>,
    /// (step name, error) for every OPTIONAL step that failed (warnings).
    pub warnings: Vec<(String, String)>,
}

impl StepReport {
    pub fn new() -> Self {
        StepReport {
            failures: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Run one transaction step. Required-step failures are recorded in
    /// `failures` (the apply must roll back); optional-step failures end up in
    /// `warnings` and never fail the apply.
    pub fn run(&mut self, name: &str, required: bool, f: impl FnOnce() -> Result<(), String>) {
        match f() {
            Ok(()) => println!("==> {name}: OK"),
            Err(e) if required => {
                eprintln!("==> {name}: FAILED — {e}");
                self.failures.push((name.to_string(), e));
            }
            Err(e) => {
                eprintln!("==> {name}: WARNING — {e}");
                self.warnings.push((name.to_string(), e));
            }
        }
    }

    /// True when no required step failed — i.e. the apply may be committed.
    pub fn required_ok(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Hidden same-volume staging dir where the payload is extracted and validated
/// BEFORE any live file is touched.
pub const STAGE_DIR: &str = ".mc-stage";
/// Hidden backup dir holding the previously installed binaries until the
/// post-install verification passes (the rollback source).
pub const BACKUP_DIR: &str = ".mc-backup";

/// Repair state left behind by a crashed/interrupted apply, then remove
/// abandoned staging directories. Runs before every apply so a dead update
/// can never leave mixed-version binaries in place.
pub fn recover_interrupted(install_dir: &Path) {
    // 1. Leftover backup → the last apply died between file swap and
    //    verification: restore the prior binaries so retries start clean.
    let backup = install_dir.join(BACKUP_DIR);
    if backup.is_dir() {
        eprintln!(
            "==> recovery: interrupted update detected ({}) — restoring previous binaries...",
            backup.display()
        );
        match restore_backup(install_dir, &backup) {
            Ok(restored) => eprintln!("    recovery: restored {restored} item(s)."),
            Err(e) => eprintln!(
                "    WARNING: backup restore failed ({e}) — current state may be partial; the next update overwrites it."
            ),
        }
        let _ = std::fs::remove_dir_all(&backup);
    }
    // 2. Abandoned staging dir without a backup → the previous apply crashed
    //    during extraction; nothing was swapped, so it is safe to delete.
    let stage = install_dir.join(STAGE_DIR);
    if stage.is_dir() {
        eprintln!(
            "==> cleanup: removing abandoned staging directory {}...",
            stage.display()
        );
        let _ = std::fs::remove_dir_all(&stage);
    }
}

/// Recursive copy helper (dirs and files).
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("create {}: {e}", dst.display()))?;
        let entries = std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))?;
        for entry in entries.flatten() {
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(p) = dst.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        std::fs::copy(src, dst)
            .map(|_| ())
            .map_err(|e| format!("copy {} -> {}: {e}", src.display(), dst.display()))
    }
}

/// Remove a file or a whole directory tree.
fn remove_tree(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        std::fs::remove_dir_all(path).map_err(|e| format!("remove dir {}: {e}", path.display()))
    } else {
        std::fs::remove_file(path).map_err(|e| format!("remove file {}: {e}", path.display()))
    }
}

/// Keep a same-volume copy of every previously installed critical file so a
/// failed apply can be rolled back. The `setup\` dir is deliberately never
/// backed up: the shell is permanent (S65-002).
fn backup_critical(install_dir: &Path, backup: &Path) -> Result<(), String> {
    if backup.exists() {
        remove_tree(backup)?;
    }
    std::fs::create_dir_all(backup)
        .map_err(|e| format!("create backup dir {}: {e}", backup.display()))?;
    for item in [
        ctx::MAIN_EXE,
        ctx::ECRAM_EXE,
        ctx::BRIDGE_EXE,
        ctx::FACE_SVC_EXE,
        ctx::FACE_CP_DLL,
        "WebView2Loader.dll",
        "drivers",
        "resources",
        "payload.json",
    ] {
        let src = install_dir.join(item);
        if src.exists() {
            copy_tree(&src, &backup.join(item))?;
        }
    }
    Ok(())
}

/// Move every backed-up item back over the live install dir.
fn restore_backup(install_dir: &Path, backup: &Path) -> Result<usize, String> {
    let names: Vec<PathBuf> = std::fs::read_dir(backup)
        .map_err(|e| format!("read backup dir {}: {e}", backup.display()))?
        .flatten()
        .map(|e| e.path())
        .collect();
    let mut restored = 0usize;
    for src in names {
        let name = src.file_name().unwrap_or_default();
        let name_str = name.to_string_lossy().to_string();
        if name_str.eq_ignore_ascii_case("setup") || name_str.starts_with(".mc-") {
            continue;
        }
        let live = install_dir.join(name);
        if live.exists() {
            remove_tree(&live)?;
        }
        std::fs::rename(&src, &live)
            .map_err(|e| format!("restore {} -> {}: {e}", src.display(), live.display()))?;
        restored += 1;
    }
    Ok(restored)
}

/// Move every staged top-level item into the live install dir (overwriting),
/// never touching `setup\` or our own `.mc-*` dirs.
fn swap_staged(install_dir: &Path, stage: &Path) -> Result<usize, String> {
    let names: Vec<PathBuf> = std::fs::read_dir(stage)
        .map_err(|e| format!("read staging dir {}: {e}", stage.display()))?
        .flatten()
        .map(|e| e.path())
        .collect();
    let mut swapped = 0usize;
    for src in names {
        let name = src.file_name().unwrap_or_default();
        let name_str = name.to_string_lossy().to_string();
        if name_str.eq_ignore_ascii_case("setup") || name_str.starts_with(".mc-") {
            continue;
        }
        let live = install_dir.join(name);
        if live.exists() {
            remove_tree(&live)?;
        }
        std::fs::rename(&src, &live)
            .map_err(|e| format!("swap {} -> {}: {e}", src.display(), live.display()))?;
        swapped += 1;
    }
    if swapped > 0 {
        let _ = std::fs::remove_dir(stage);
    }
    Ok(swapped)
}

/// Clean the staging dir plus any downloaded payload on an early abort.
fn cleanup_staged(resolved: &payload::ResolvedPayload, install_dir: &Path) {
    let _ = std::fs::remove_dir_all(install_dir.join(STAGE_DIR));
    cleanup_downloaded(resolved);
}

/// Apply a payload over `install_dir`.
///
/// * `opts` — CLI options (payload path/url/repo/silent/no-relaunch).
/// * `first_install` — true on `install` (WebView2, shortcuts, uninstall
///   registration); false on `update` (app + services already configured,
///   just refresh files + points).
pub fn apply_install(opts: &Options, install_dir: &Path, first_install: bool) -> i32 {
    println!("==> MiControl setup shell v{}", env!("CARGO_PKG_VERSION"));
    println!(
        "==> mode: {}",
        if first_install { "install" } else { "update" }
    );

    // ── 1. Interrupted-update recovery (S65-005) ────────────────────────────
    recover_interrupted(install_dir);

    // ── 2. Resolve the payload ──────────────────────────────────────────────
    let repo = opts.repo.as_deref().unwrap_or(ctx::DEFAULT_REPO);
    let resolved = match payload::resolve(opts.payload.as_deref(), opts.url.as_deref(), repo) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return crate::EXIT_FAIL;
        }
    };

    // ── 3. S65-004: fail-closed preflight BEFORE any disruption ─────────────
    // Validate the payload zip (required entries, safe paths, version) while
    // the app and services are still running. A bad payload must never leave
    // the installed app in a half-stopped state.
    println!("==> preflight: validating payload contents...");
    let pre = match payload::preflight(&resolved.zip) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: preflight failed: {e}");
            cleanup_downloaded(&resolved);
            return crate::EXIT_FAIL;
        }
    };
    println!("    preflight: {}", payload::preflight_summary(&pre));
    if !pre.ok {
        eprintln!(
            "error: payload rejected by preflight — missing required entries: {:?}{}{}",
            pre.missing_required,
            if pre.unsafe_path_found {
                " / unsafe paths detected"
            } else {
                ""
            },
            if pre.duplicate_path_found {
                " / duplicate paths detected"
            } else {
                ""
            }
        );
        cleanup_downloaded(&resolved);
        return crate::EXIT_FAIL;
    }

    // S65-004: version-mismatch guard. When the caller resolved the payload
    // under a specific expected version (--expect-version, e.g. from the
    // `MiControl_<ver>_payload.zip` name or the updater URL), the manifest
    // MUST agree — otherwise refuse BEFORE stopping the app/services.
    if let Some(expected) = opts.expect_version.as_deref() {
        if let Some(mismatch) = payload::version_mismatch(expected, &pre) {
            eprintln!("error: {mismatch}. Refusing to apply the payload (fail-closed).");
            cleanup_downloaded(&resolved);
            return crate::EXIT_FAIL;
        }
    }

    if let Some(sig) = &resolved.sig {
        println!("==> verifying payload signature...");
        if let Err(e) = payload::verify_signature(&resolved.zip, sig) {
            eprintln!("error: {e}");
            cleanup_downloaded(&resolved);
            return crate::EXIT_FAIL;
        }
        println!("    signature OK.");
    } else if !opts.allow_unsigned {
        eprintln!(
            "error: payload has no .sig next to it and --allow-unsigned was not given.\n\
             Refusing to apply an unverified payload (same trust model as the updater)."
        );
        return crate::EXIT_FAIL;
    }

    // ── 4. Stage the payload on the SAME VOLUME (S65-005 AC1) ───────────────
    // Extraction never runs against the live dir: the staged copy is validated
    // first, and only then swapped in.
    let stage = install_dir.join(STAGE_DIR);
    if stage.exists() {
        let _ = remove_tree(&stage);
    }
    if let Err(e) = std::fs::create_dir_all(&stage) {
        eprintln!("error: cannot create staging dir {}: {e}", stage.display());
        cleanup_downloaded(&resolved);
        return crate::EXIT_FAIL;
    }
    println!(
        "==> extracting payload to staging dir {}...",
        stage.display()
    );
    let (count, zip_version) = match payload::extract(&resolved.zip, &stage) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            cleanup_staged(&resolved, install_dir);
            return crate::EXIT_FAIL;
        }
    };
    println!("    staged {count} files.");
    if let Some(v) = &zip_version {
        println!("    staged payload version: {v}");
    }

    // Staged-content validation before any live disruption.
    let app_exe = install_dir.join(ctx::MAIN_EXE);
    let staged_app = stage.join(ctx::MAIN_EXE);
    if !staged_app.is_file() {
        eprintln!("error: payload did not contain micontrol.exe — aborting.");
        cleanup_staged(&resolved, install_dir);
        return crate::EXIT_FAIL;
    }

    // ── 5. Transaction: stop → backup → swap → install points → verify ──────
    let mut report = StepReport::new();
    let backup = install_dir.join(BACKUP_DIR);

    report.run("stop running instances", true, || {
        stop_everything(install_dir)
    });
    if !report.required_ok() {
        eprintln!("error: could not quiesce running processes — no files were changed.");
        cleanup_staged(&resolved, install_dir);
        return crate::EXIT_FAIL;
    }

    // Backup the previously installed critical files (rollback source).
    report.run("backup previous binaries", true, || {
        backup_critical(install_dir, &backup)
    });

    // Swap the validated staging content into the live dir.
    report.run("swap staged payload into place", true, || {
        let n = swap_staged(install_dir, &stage)?;
        println!("    {n} item(s) swapped into the install dir.");
        Ok(())
    });

    // Install points, old→new (preserving the legacy order). Required steps
    // MUST succeed or the whole update rolls back.
    report.run("run-key autostart", false, || write_run_key(&app_exe));

    report.run("driver VirtualControlHID", true, || {
        install_driver(
            install_dir,
            "drivers\\VirtualControlHID\\virtualcontrolhid.inf",
            "VirtualControlHID",
        )
    });
    report.run("driver IoTDriver", true, || {
        install_driver(
            install_dir,
            "drivers\\IoTDriver\\iotdriver.inf",
            "IoTDriver",
        )
    });

    report.run("ecram_service / IoTSvc deploy", true, || {
        deploy_ecram_service(install_dir)
    });

    report.run("MiControlElevated scheduled task", true, || {
        register_elevated_task(&app_exe)
    });

    report.run("MiControlBridge service", true, || {
        install_bridge_service(install_dir)
    });

    report.run("Face Unlock", false, || install_face_unlock(install_dir));

    // ── 6. Post-install verification (critical gate) ────────────────────────
    report.run("post-install verification", true, || {
        verify_services(install_dir)
    });

    if !report.required_ok() {
        // ── 7. ROLLBACK: restore binaries + critical points, report both ────
        return rollback_and_report(install_dir, &backup, &report, &resolved);
    }

    // ── 8. Commit: verification passed, drop the rollback artifacts ─────────
    let _ = remove_tree(&backup);
    let _ = remove_tree(&stage);
    cleanup_downloaded(&resolved);

    // Legacy NSIS uninstaller is superseded by this shell.
    let legacy_uninstaller = install_dir.join("uninstall.exe");
    if legacy_uninstaller.is_file() {
        let _ = std::fs::remove_file(&legacy_uninstaller);
    }

    // ── 9. First-install extras + relaunch (optional steps) ─────────────────
    if first_install {
        let mut extra = StepReport::new();
        extra.run("WebView2 runtime", false, ensure_webview2);
        extra.run("uninstaller registration", false, || {
            register_uninstaller(install_dir, zip_version.as_deref())
        });
        extra.run("shortcuts", false, || create_shortcuts(&app_exe));
    } else if let Some(v) = zip_version.as_deref() {
        // Keep DisplayVersion in sync with the newly applied payload.
        let _ = update_uninstaller_version(install_dir, Some(v));
    }

    if !opts.no_relaunch {
        let mut extra = StepReport::new();
        extra.run("relaunch MiControl", false, || {
            sys::launch_app_for_user(&app_exe)
        });
    }

    println!("==> done.");
    crate::EXIT_OK
}

/// Restore the previous installation after required-step failures, then
/// report BOTH the original failures and the rollback outcome (S65-005 AC4).
fn rollback_and_report(
    install_dir: &Path,
    backup: &Path,
    report: &StepReport,
    resolved: &payload::ResolvedPayload,
) -> i32 {
    eprintln!();
    eprintln!(
        "==> ROLLBACK: update failed on {} required step(s); restoring the previous installation...",
        report.failures.len()
    );
    for (name, e) in &report.failures {
        eprintln!("    original failure [{name}]: {e}");
    }
    let mut rb = StepReport::new();
    rb.run("stop running instances (rollback)", true, || {
        stop_everything(install_dir)
    });
    rb.run("restore previous binaries", true, || {
        let n = restore_backup(install_dir, backup)?;
        println!("    restored {n} item(s).");
        Ok(())
    });
    // Re-apply the critical install points against the RESTORED binaries so
    // the previous installation is fully usable again.
    rb.run("restore bridge service", true, || {
        install_bridge_service(install_dir)
    });
    rb.run("restore ecram/IoTSvc", true, || {
        deploy_ecram_service(install_dir)
    });
    rb.run("restore elevated task", true, || {
        register_elevated_task(&install_dir.join(ctx::MAIN_EXE))
    });
    rb.run("verify rollback", true, || verify_services(install_dir));
    let _ = remove_tree(backup);
    let _ = remove_tree(&install_dir.join(STAGE_DIR));
    cleanup_downloaded(resolved);
    eprintln!();
    if rb.required_ok() {
        eprintln!(
            "==> ROLLBACK COMPLETE — the previous installation was restored ({} failure(s) during the update).",
            report.failures.len()
        );
    } else {
        eprintln!(
            "==> ROLLBACK PARTIAL — previous binaries restored, but {} rollback step(s) also failed.",
            rb.failures.len()
        );
        for (n, e) in &rb.failures {
            eprintln!("    rollback failure [{n}]: {e}");
        }
        eprintln!("    Please reinstall MiControl manually to restore full service state.");
    }
    crate::EXIT_FAIL
}

fn cleanup_downloaded(resolved: &payload::ResolvedPayload) {
    if resolved.downloaded {
        let _ = std::fs::remove_file(&resolved.zip);
        if let Some(sig) = &resolved.sig {
            let _ = std::fs::remove_file(sig);
        }
    }
}

/// Stop the app, the bridge service, the face service and the IoT service so
/// the live files are never locked when the payload is swapped in.
///
/// This runs INSIDE the shell process, which none of these processes is —
/// the structural advantage of the decoupled installer: no self-kill, ever.
/// Returns Err only when a process/service is still alive after its bounded
/// wait, i.e. when the file swap could NOT be performed safely.
fn stop_everything(install_dir: &Path) -> Result<(), String> {
    fail_check("stop")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }

    let mut problems: Vec<String> = Vec::new();

    // App (interactive) — kill + wait so the exe is replaceable.
    if !sys::kill_and_wait(ctx::MAIN_EXE, Duration::from_secs(10)) {
        problems.push(format!("{} did not exit", ctx::MAIN_EXE));
    }

    // Bridge service: stop (file must be replaceable). The bridge entry is
    // re-created later by `micontrol_bridge.exe install`, which is itself
    // self-healing against SCM delete races (v0.1.21/v0.1.22 lessons).
    if !sys::service_stop_wait(ctx::APP_SERVICE, Duration::from_secs(20)) {
        problems.push(format!("{} did not stop", ctx::APP_SERVICE));
    }

    // Face service: stop so micontrol_face_svc.exe is replaceable.
    if !sys::service_stop_wait(ctx::FACE_SERVICE, Duration::from_secs(15)) {
        problems.push(format!("{} did not stop", ctx::FACE_SERVICE));
    }

    // IoTService / ecram_service: killed again inside the DriverStore block,
    // but free them early so the ecram_service.exe swap cannot fail.
    let _ = sys::run_capture(
        "sc.exe",
        &["failure", ctx::IOT_SERVICE, "reset=", "0", "actions=", ""],
    );
    if !sys::service_stop_wait(ctx::IOT_SERVICE, Duration::from_secs(15)) {
        problems.push(format!("{} did not stop", ctx::IOT_SERVICE));
    }
    if !sys::kill_and_wait("IoTService.exe", Duration::from_secs(10)) {
        problems.push("IoTService.exe did not exit".to_string());
    }
    if !sys::kill_and_wait(ctx::ECRAM_EXE, Duration::from_secs(10)) {
        problems.push(format!("{} did not exit", ctx::ECRAM_EXE));
    }

    let _ = install_dir;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

fn write_run_key(app_exe: &Path) -> Result<(), String> {
    fail_check("run-key")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_SET_VALUE};
    let hk = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hk
        .open_subkey_with_flags(ctx::RUN_KEY_PATH, KEY_SET_VALUE)
        .map_err(|e| format!("open {}: {e}", ctx::RUN_KEY_PATH))?;
    key.set_value("MiControl", &format!("\"{}\"", app_exe.display()))
        .map_err(|e| format!("set Run value: {e}"))
}

fn install_driver(install_dir: &Path, rel_inf: &str, label: &str) -> Result<(), String> {
    fail_check(&format!("driver:{label}"))?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    let inf = install_dir.join(rel_inf);
    if !inf.is_file() {
        println!("==> {label}: INF not found ({}) — skipping.", inf.display());
        return Ok(());
    }
    println!("==> driver {label}: pnputil /add-driver ...");
    let (code, out) = sys::run_capture(
        "pnputil.exe",
        &["/add-driver", &inf.display().to_string(), "/install"],
    );
    match code {
        0 => {
            println!("    {label}: installed successfully.");
            Ok(())
        }
        3010 => {
            println!("    {label}: installed — reboot required to activate.");
            Ok(())
        }
        _ => Err(format!(
            "{label}: pnputil returned {code} — review required (reboot may be needed).\n{out}"
        )),
    }
}

/// Locate the IoTDriver package directory in the DriverStore (lexicographically
/// greatest = most recently staged package).
pub fn driverstore_package_dir() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
        .join(r"System32\DriverStore\FileRepository");
    let entries = std::fs::read_dir(&root).ok()?;
    let mut best: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("iotdriver.inf_") && entry.path().is_dir() {
            // Lexicographically greatest = most recently staged package.
            if best.as_ref().is_none_or(|b| entry.path() > *b) {
                best = Some(entry.path());
            }
        }
    }
    best
}

fn deploy_ecram_service(install_dir: &Path) -> Result<(), String> {
    fail_check("ecram")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    println!("==> deploying ecram_service into the DriverStore...");
    let Some(pkg) = driverstore_package_dir() else {
        return Err(
            "iotdriver.inf_* package not found in DriverStore — ecram_service not deployed".into(),
        );
    };
    println!("    IoTDriver package: {}", pkg.display());

    let source = install_dir.join(ctx::ECRAM_EXE);
    if !source.is_file() {
        return Err(format!(
            "payload does not contain {} — IoTSvc not deployed",
            ctx::ECRAM_EXE
        ));
    }
    let dest = pkg.join("IoTService.exe");

    // Stop + neutralize the old service before replacing the binary
    // (failure actions would auto-restart it and re-lock the file).
    let _ = sys::run_capture(
        "sc.exe",
        &["failure", ctx::IOT_SERVICE, "reset=", "0", "actions=", ""],
    );
    sys::service_stop_wait(ctx::IOT_SERVICE, Duration::from_secs(15));
    sys::kill_and_wait("IoTService.exe", Duration::from_secs(10));
    sys::kill_and_wait(ctx::ECRAM_EXE, Duration::from_secs(10));

    // Backup once (secondary rollback point inside the DriverStore).
    if dest.is_file() {
        let bak = PathBuf::from(format!("{}.bak", dest.display()));
        if !bak.exists() {
            let _ = std::fs::copy(&dest, &bak);
        }
    }

    std::fs::copy(&source, &dest).map_err(|e| {
        format!(
            "failed to copy ecram_service.exe to {}: {e} — IoTSvc was NOT modified",
            dest.display()
        )
    })?;
    println!("    ecram_service deployed -> {}", dest.display());

    // Remove the stale entry (best-effort — SCM deletes are async; the
    // v0.1.22 lesson is that a slow delete must NEVER abort the flow).
    sys::service_delete_best_effort(ctx::IOT_SERVICE);

    // `ecram_service.exe install-service` (Rust) creates the service with the
    // correct QUOTED binPath + `service` argument, starts it and verifies
    // RUNNING — the only form that reliably reaches RUNNING (repo lesson).
    let (code, out) = sys::run_capture(
        &dest.display().to_string(),
        &["install-service", &dest.display().to_string()],
    );
    if code == 0 {
        println!("    IoTSvc recreated and RUNNING with ecram_service (verified).");
        Ok(())
    } else {
        Err(format!(
            "ecram_service install-service returned {code} — service NOT recreated.\n{out}"
        ))
    }
}

fn register_elevated_task(app_exe: &Path) -> Result<(), String> {
    fail_check("task")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    println!("==> registering MiControlElevated scheduled task...");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><TimeTrigger><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>false</Enabled></TimeTrigger></Triggers><Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT120S</ExecutionTimeLimit><Enabled>true</Enabled></Settings><Actions Context="Author"><Exec><Command>"{}"</Command><Arguments>--elevated</Arguments></Exec></Actions></Task>"#,
        app_exe.display()
    );
    let xml_path = std::env::temp_dir().join("MCElev.xml");
    std::fs::write(&xml_path, xml).map_err(|e| format!("could not write task XML: {e}"))?;
    let _ = sys::run_capture("schtasks.exe", &["/delete", "/tn", ctx::ELEV_TASK, "/f"]);
    let (code, out) = sys::run_capture(
        "schtasks.exe",
        &[
            "/create",
            "/tn",
            ctx::ELEV_TASK,
            "/xml",
            &xml_path.display().to_string(),
            "/f",
        ],
    );
    let _ = std::fs::remove_file(&xml_path);
    if code == 0 {
        println!("    MiControlElevated task registered: OK");
        Ok(())
    } else {
        Err(format!(
            "schtasks /create MiControlElevated returned {code} — the app would self-heal on first run, but the task is required.\n{out}"
        ))
    }
}

fn install_bridge_service(install_dir: &Path) -> Result<(), String> {
    fail_check("bridge")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    println!("==> installing MiControlBridge service (autonomous elevated bridge)...");
    let bridge = install_dir.join(ctx::BRIDGE_EXE);
    if !bridge.is_file() {
        return Err(format!(
            "{} not in payload — MiControlBridge service not installed",
            ctx::BRIDGE_EXE
        ));
    }
    let (code, out) = sys::run_capture(&bridge.display().to_string(), &["install"]);
    if code == 0 {
        println!("    MiControlBridge service installed: OK");
        return Ok(());
    }
    println!(
        "    micontrol_bridge install returned {code} — trying direct sc.exe fallback.\n    {out}"
    );
    // Fallback chain (port of the NSIS hook): best-effort delete-wait, then
    // direct create + failure-actions + start. Worst case is recoverable:
    // the app self-heals on next start (ensure_bridge_service).
    for _ in 0..15 {
        if !sys::service_exists(ctx::APP_SERVICE) {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let bin_path = format!("binPath= \"\\\"{}\\\" service\"", bridge.display());
    let _ = sys::run_capture(
        "sc.exe",
        &[
            "create",
            ctx::APP_SERVICE,
            &bin_path,
            "start=",
            "auto",
            "DisplayName=",
            "MiControl Bridge Service",
        ],
    );
    let _ = sys::run_capture(
        "sc.exe",
        &[
            "failure",
            ctx::APP_SERVICE,
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/10000/restart/30000",
        ],
    );
    let _ = sys::run_capture("sc.exe", &["start", ctx::APP_SERVICE]);
    if sys::service_exists(ctx::APP_SERVICE) {
        println!("    MiControlBridge service created via direct sc.exe fallback.");
        Ok(())
    } else {
        Err("MiControlBridge could not be created via the sc.exe fallback either".into())
    }
}

fn install_face_unlock(install_dir: &Path) -> Result<(), String> {
    fail_check("face")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    println!("==> setting up Face Unlock...");
    let face_svc = install_dir.join(ctx::FACE_SVC_EXE);
    let face_cp = install_dir.join(ctx::FACE_CP_DLL);

    if face_svc.is_file() {
        // Delete the stale entry first (in-place change_config fails with
        // ERROR_SERVICE_MARKED_FOR_DELETE on slow shutdowns — repo lesson).
        sys::service_delete_best_effort(ctx::FACE_SERVICE);
        let (code, out) = sys::run_capture(&face_svc.display().to_string(), &["install"]);
        if code == 0 {
            println!("    MiControlFace service install: OK");
        } else {
            println!(
                "    WARNING: MiControlFace service install returned {code} (Face Unlock remains unavailable on this upgrade).\n    {out}"
            );
        }
    } else {
        println!("    Note: micontrol_face_svc.exe not found in payload — Face Unlock disabled.");
    }

    if face_cp.is_file() {
        use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_LOCAL_MACHINE};
        let clsid = r"SOFTWARE\Classes\CLSID\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}";
        let provider = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}";

        let hk = winreg::RegKey::predef(HKEY_CLASSES_ROOT);
        if let Ok(key) = hk.create_subkey(clsid) {
            let (key, _) = key;
            let _ = key.set_value("", &"MiControl Face Unlock");
            if let Ok(inproc) = key.create_subkey("InprocServer32") {
                let (inproc, _) = inproc;
                let _ = inproc.set_value("", &face_cp.display().to_string());
                let _ = inproc.set_value("ThreadingModel", &"Apartment");
            }
            let _ = key.set_value("DisableModeless", &1u32);
        }
        let hklm = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(key) = hklm.create_subkey(provider) {
            let (key, _) = key;
            let _ = key.set_value("", &"MiControl Face Unlock");
        }
        println!("    micontrol_facecp.dll registered as Credential Provider (registry).");
    }

    let face_dir = std::env::var("ProgramData")
        .map(|p| PathBuf::from(p).join(r"MiControl\face"))
        .unwrap_or_else(|_| PathBuf::from(r"C:\ProgramData\MiControl\face"));
    let _ = std::fs::create_dir_all(face_dir);
    println!("    Face Unlock configured.");
    Ok(())
}

/// Post-install verification (the critical gate of the transaction). The
/// MiControlBridge service MUST exist — that one defect never commits.
fn verify_services(install_dir: &Path) -> Result<(), String> {
    fail_check("verify")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    println!("==> verifying installed services...");
    let mut problems: Vec<String> = Vec::new();
    if sys::service_exists(ctx::APP_SERVICE) {
        println!("    MiControlBridge present (sc query OK).");
    } else {
        problems.push("MiControlBridge did not respond to sc query".to_string());
    }
    if install_dir.join(ctx::FACE_SVC_EXE).is_file() {
        std::thread::sleep(Duration::from_millis(2500));
        if sys::service_is_running(ctx::FACE_SERVICE) {
            println!("    MiControlFace is RUNNING.");
        } else {
            println!("    WARNING: MiControlFace is not RUNNING after installation. Check C:\\ProgramData\\MiControl\\face\\face_svc.log");
        }
    }
    println!("    Service verification complete.");
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

fn ensure_webview2() -> Result<(), String> {
    fail_check("webview2")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY};
    const KEYS: [&str; 3] = [
        r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients",
        r"SOFTWARE\Microsoft\EdgeUpdate\Clients",
        r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\User",
    ];
    for base in KEYS {
        let hk = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
        for flags in [KEY_READ, KEY_READ | KEY_WOW64_32KEY] {
            if let Ok(key) = hk.open_subkey_with_flags(base, flags) {
                if let Ok(pv) = key.get_value::<String, _>(ctx::WEBVIEW2_GUID) {
                    if !pv.trim().is_empty() {
                        println!("==> WebView2 runtime present ({pv}).");
                        return Ok(());
                    }
                }
            }
        }
    }
    println!("==> WebView2 runtime NOT found — downloading bootstrapper...");
    let boot = std::env::temp_dir().join("MicrosoftEdgeWebview2Setup.exe");
    match payload::download_public("https://go.microsoft.com/fwlink/?linkid=2124701", &boot) {
        Ok(()) => {
            let exe = boot.display().to_string();
            let (code, _) = sys::run_capture(&exe, &["/silent", "/install"]);
            println!(
                "    WebView2 bootstrapper exit {code}{}",
                if code == 0 {
                    "."
                } else {
                    " — the app will prompt to install WebView2 at first run."
                }
            );
            Ok(())
        }
        Err(e) => Err(format!("WebView2 bootstrapper download failed: {e}")),
    }
}

const ESTIMATED_SIZE_UNIT_KB: u64 = 1024;

fn dir_size_bytes(dir: &Path) -> u64 {
    fn walk(dir: &Path, total: &mut u64) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    walk(&entry.path(), total);
                } else {
                    *total += meta.len();
                }
            }
        }
    }
    let mut total = 0;
    walk(dir, &mut total);
    total
}

fn register_uninstaller(install_dir: &Path, version: Option<&str>) -> Result<(), String> {
    fail_check("uninstaller")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let shell = ctx::shell_path(install_dir);
    let uninstall_cmd = format!("\"{}\" uninstall", shell.display());
    let quiet_cmd = format!("\"{}\" uninstall --silent", shell.display());
    let estimated_kb = dir_size_bytes(install_dir) / ESTIMATED_SIZE_UNIT_KB;

    let hk = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok((key, _)) = hk.create_subkey(ctx::UNINST_KEY_PATH) else {
        return Err(format!(
            "could not create the uninstall registry key at {}",
            ctx::UNINST_KEY_PATH
        ));
    };
    let _ = key.set_value("DisplayName", &"MiControl");
    if let Some(v) = version {
        let _ = key.set_value("DisplayVersion", &v);
    }
    let _ = key.set_value("Publisher", &"MiControl");
    let _ = key.set_value("InstallLocation", &install_dir.display().to_string());
    let _ = key.set_value(
        "DisplayIcon",
        &format!("\"{}\"", install_dir.join(ctx::MAIN_EXE).display()),
    );
    let _ = key.set_value("UninstallString", &uninstall_cmd);
    let _ = key.set_value("QuietUninstallString", &quiet_cmd);
    let _ = key.set_value("NoModify", &1u32);
    let _ = key.set_value("NoRepair", &1u32);
    let _ = key.set_value("EstimatedSize", &(estimated_kb as u32));
    println!("==> uninstaller registered (shell-based).");
    Ok(())
}

fn update_uninstaller_version(install_dir: &Path, version: Option<&str>) -> Result<(), String> {
    fail_check("uninstaller-version")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_SET_VALUE};
    let Some(version) = version else {
        return Ok(());
    };
    let hk = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(key) = hk.open_subkey_with_flags(ctx::UNINST_KEY_PATH, KEY_SET_VALUE) {
        let _ = key.set_value("DisplayVersion", &version);
        Ok(())
    } else {
        // No key yet (upgrading from a legacy install) → full registration.
        register_uninstaller(install_dir, Some(version))
    }
}

fn create_shortcuts(app_exe: &Path) -> Result<(), String> {
    fail_check("shortcuts")?;
    #[cfg(test)]
    if *SYSTEM_NOOP.lock().unwrap() {
        return Ok(());
    }
    println!("==> creating shortcuts...");
    let mut problems: Vec<String> = Vec::new();
    for dest in [
        crate::shortcut::desktop_shortcut_path(),
        crate::shortcut::start_menu_shortcut_path(),
    ] {
        match crate::shortcut::create_shortcut(app_exe, &dest, "MiControl") {
            Ok(()) => println!("    shortcut: {}", dest.display()),
            Err(e) => problems.push(format!("shortcut {} failed: {e}", dest.display())),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Version string recorded in the installed payload.json.
pub fn read_payload_version(install_dir: &Path) -> Option<String> {
    payload::installed_version(install_dir)
}

// ── S65-005 tests: transactional apply, rollback, recovery, idempotency ──────
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Serialize ALL tests in this module: they share the global SYSTEM_NOOP /
    /// FAIL_POINTS hooks, so parallel execution leaks injected failures
    /// between tests (observed: rollback tests failing only under the default
    /// parallel runner). One lock, taken for the whole test body, makes the
    /// suite deterministic under `cargo test` with any thread count.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that holds TEST_LOCK for the duration of a test.
    /// The field is never read — the guard's mere existence (and its Drop)
    /// is the whole point.
    #[allow(dead_code)]
    struct TestGuard<'a>(std::sync::MutexGuard<'a, ()>);

    impl<'a> TestGuard<'a> {
        fn acquire() -> Self {
            Self(TEST_LOCK.lock().unwrap())
        }
    }

    impl Drop for TestGuard<'_> {
        fn drop(&mut self) {
            // Reset the shared hooks so a panicking test cannot poison the
            // next one's starting state.
            set_system_noop(false);
            clear_failures();
        }
    }

    static ZIP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn make_zip(dir: &Path, entries: &[(&str, &str)], manifest: Option<&str>) -> PathBuf {
        let n = ZIP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let zip_path = dir.join(format!("payload-{n}.zip"));
        let file = fs::File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(*name, opts).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        if let Some(m) = manifest {
            writer.start_file("payload.json", opts).unwrap();
            writer.write_all(m.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        zip_path
    }

    fn valid_payload_zip(dir: &Path) -> PathBuf {
        make_zip(
            dir,
            &[
                ("micontrol.exe", "NEW-APP"),
                ("ecram_service.exe", "NEW-ECRAM"),
                ("micontrol_bridge.exe", "NEW-BRIDGE"),
                ("micontrol_face_svc.exe", "NEW-FACE"),
                ("WebView2Loader.dll", "NEW-WV2"),
                ("drivers/VirtualControlHID/virtualcontrolhid.inf", "; inf"),
                ("drivers/IoTDriver/iotdriver.inf", "; inf"),
                ("resources/empty.txt", ""),
            ],
            Some(r#"{"version":"1.2.3"}"#),
        )
    }

    /// A bogus previous installation: old binaries, a permanent `setup\`
    /// shell and a user file that must survive every apply/recover.
    fn old_install(dir: &Path) -> PathBuf {
        let install = dir.join(format!(
            "install-{}",
            ZIP_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(install.join("setup")).unwrap();
        fs::write(install.join("micontrol.exe"), "OLD-APP").unwrap();
        fs::write(install.join("ecram_service.exe"), "OLD-ECRAM").unwrap();
        fs::write(install.join("micontrol_bridge.exe"), "OLD-BRIDGE").unwrap();
        fs::write(install.join("micontrol_face_svc.exe"), "OLD-FACE").unwrap();
        fs::write(install.join("setup/micontrol_setup.exe"), "SHELL-SENTINEL").unwrap();
        fs::write(install.join("user-data.txt"), "USER").unwrap();
        install
    }

    fn opts_for(dir: &Path, payload: &Path) -> Options {
        Options {
            payload: Some(payload.to_path_buf()),
            url: None,
            dir: Some(dir.to_path_buf()),
            repo: None,
            silent: false,
            no_relaunch: true,
            allow_unsigned: true,
            expect_version: None,
        }
    }

    fn read_str(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    fn temp_root(tag: &str) -> PathBuf {
        let td = std::env::temp_dir().join(format!("s65-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&td);
        fs::create_dir_all(&td).unwrap();
        td
    }

    #[test]
    fn apply_success_swaps_staged_payload_and_keeps_shell_and_user_files() {
        let _guard = TestGuard::acquire();
        set_system_noop(true);
        clear_failures();
        let td = temp_root("ok");
        let install = old_install(&td);
        let zip = valid_payload_zip(&td);
        let opts = opts_for(&install, &zip);

        let code = apply_install(&opts, &install, false);

        assert_eq!(code, crate::EXIT_OK);
        assert_eq!(read_str(&install.join("micontrol.exe")), "NEW-APP");
        assert_eq!(read_str(&install.join("ecram_service.exe")), "NEW-ECRAM");
        assert_eq!(
            read_str(&install.join("micontrol_bridge.exe")),
            "NEW-BRIDGE"
        );
        assert_eq!(read_str(&install.join("WebView2Loader.dll")), "NEW-WV2");
        // Shell territory + user files untouched.
        assert_eq!(
            read_str(&install.join("setup/micontrol_setup.exe")),
            "SHELL-SENTINEL"
        );
        assert_eq!(read_str(&install.join("user-data.txt")), "USER");
        // Transaction committed: rollback artifacts are gone.
        assert!(!install.join(BACKUP_DIR).exists());
        assert!(!install.join(STAGE_DIR).exists());

        // Idempotent: a second apply over the updated install also succeeds
        // and never touches the permanent shell.
        let code2 = apply_install(&opts, &install, false);
        assert_eq!(code2, crate::EXIT_OK);
        assert_eq!(
            read_str(&install.join("setup/micontrol_setup.exe")),
            "SHELL-SENTINEL"
        );
        assert!(!install.join(BACKUP_DIR).exists());
        assert!(!install.join(STAGE_DIR).exists());
        let _ = fs::remove_dir_all(&td);
    }

    #[test]
    fn required_step_failure_rolls_back_previous_binaries() {
        let _guard = TestGuard::acquire();
        set_system_noop(true);
        clear_failures();
        inject_failure("bridge");
        let td = temp_root("rb");
        let install = old_install(&td);
        let zip = valid_payload_zip(&td);
        let opts = opts_for(&install, &zip);

        let code = apply_install(&opts, &install, false);

        // A required-step failure must never be reported as success.
        assert_eq!(code, crate::EXIT_FAIL);
        // Previous usable binaries restored; shell + user files preserved.
        assert_eq!(read_str(&install.join("micontrol.exe")), "OLD-APP");
        assert_eq!(
            read_str(&install.join("micontrol_bridge.exe")),
            "OLD-BRIDGE"
        );
        assert_eq!(read_str(&install.join("ecram_service.exe")), "OLD-ECRAM");
        assert_eq!(
            read_str(&install.join("micontrol_face_svc.exe")),
            "OLD-FACE"
        );
        assert_eq!(
            read_str(&install.join("setup/micontrol_setup.exe")),
            "SHELL-SENTINEL"
        );
        assert_eq!(read_str(&install.join("user-data.txt")), "USER");
        // Rollback artifacts cleaned up — retry lands on a clean state.
        assert!(!install.join(BACKUP_DIR).exists());
        assert!(!install.join(STAGE_DIR).exists());
        let _ = fs::remove_dir_all(&td);
        clear_failures();
    }

    #[test]
    fn verification_failure_rolls_back_previous_binaries() {
        let _guard = TestGuard::acquire();
        set_system_noop(true);
        clear_failures();
        inject_failure("verify");
        let td = temp_root("vf");
        let install = old_install(&td);
        let zip = valid_payload_zip(&td);
        let opts = opts_for(&install, &zip);

        let code = apply_install(&opts, &install, false);

        assert_eq!(code, crate::EXIT_FAIL);
        assert_eq!(read_str(&install.join("micontrol.exe")), "OLD-APP");
        assert_eq!(
            read_str(&install.join("setup/micontrol_setup.exe")),
            "SHELL-SENTINEL"
        );
        assert!(!install.join(BACKUP_DIR).exists());
        let _ = fs::remove_dir_all(&td);
        clear_failures();
    }

    #[test]
    fn optional_step_failure_does_not_fail_the_update() {
        let _guard = TestGuard::acquire();
        set_system_noop(true);
        clear_failures();
        inject_failure("face");
        let td = temp_root("of");
        let install = old_install(&td);
        let zip = valid_payload_zip(&td);
        let opts = opts_for(&install, &zip);

        let code = apply_install(&opts, &install, false);

        // Face Unlock is optional: the update still commits.
        assert_eq!(code, crate::EXIT_OK);
        assert_eq!(read_str(&install.join("micontrol.exe")), "NEW-APP");
        assert!(!install.join(BACKUP_DIR).exists());
        let _ = fs::remove_dir_all(&td);
        clear_failures();
    }

    #[test]
    fn interrupted_update_recovery_restores_leftover_backup() {
        let _guard = TestGuard::acquire();
        set_system_noop(true);
        clear_failures();
        let td = temp_root("rec");
        let install = old_install(&td);
        // Simulate a crash between swap and verification: the backup holds the
        // OLD binaries, the live dir already holds the NEW (mixed) ones.
        fs::create_dir_all(install.join(BACKUP_DIR)).unwrap();
        fs::write(install.join(BACKUP_DIR).join("micontrol.exe"), "OLD-APP").unwrap();
        fs::write(
            install.join(BACKUP_DIR).join("micontrol_bridge.exe"),
            "OLD-BRIDGE",
        )
        .unwrap();
        fs::write(install.join("micontrol.exe"), "NEW-APP").unwrap();

        recover_interrupted(&install);

        // Prior state restored; backup removed; shell untouched.
        assert_eq!(read_str(&install.join("micontrol.exe")), "OLD-APP");
        assert_eq!(
            read_str(&install.join("micontrol_bridge.exe")),
            "OLD-BRIDGE"
        );
        assert!(!install.join(BACKUP_DIR).exists());
        assert_eq!(
            read_str(&install.join("setup/micontrol_setup.exe")),
            "SHELL-SENTINEL"
        );
        let _ = fs::remove_dir_all(&td);
    }

    #[test]
    fn abandoned_staging_directory_is_cleaned_up() {
        let _guard = TestGuard::acquire();
        set_system_noop(true);
        clear_failures();
        let td = temp_root("ab");
        let install = old_install(&td);
        fs::create_dir_all(install.join(STAGE_DIR)).unwrap();
        fs::write(install.join(STAGE_DIR).join("micontrol.exe"), "HALF").unwrap();

        recover_interrupted(&install);

        assert!(!install.join(STAGE_DIR).exists());
        // The live install is unaffected by the crashed extraction.
        assert_eq!(read_str(&install.join("micontrol.exe")), "OLD-APP");
        let _ = fs::remove_dir_all(&td);
    }

    #[test]
    fn staged_payload_without_micontrol_exe_is_rejected_before_any_disruption() {
        let _guard = TestGuard::acquire();
        set_system_noop(true);
        clear_failures();
        let td = temp_root("noexe");
        let install = old_install(&td);
        // Payload that passes preflight but actually lacks micontrol.exe.
        let zip = make_zip(
            &td,
            &[("ecram_service.exe", "X"), ("micontrol_bridge.exe", "X")],
            Some(r#"{"version":"1.2.3"}"#),
        );
        let opts = opts_for(&install, &zip);
        fs::write(install.join("micontrol.exe"), "OLD-APP").unwrap();

        let code = apply_install(&opts, &install, false);

        assert_eq!(code, crate::EXIT_FAIL);
        // Nothing was disrupted.
        assert_eq!(read_str(&install.join("micontrol.exe")), "OLD-APP");
        assert_eq!(
            read_str(&install.join("setup/micontrol_setup.exe")),
            "SHELL-SENTINEL"
        );
        assert!(!install.join(STAGE_DIR).exists());
        let _ = fs::remove_dir_all(&td);
    }
}
