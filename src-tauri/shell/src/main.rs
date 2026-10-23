//! MiControl static setup shell.
//!
//! Decoupled installer architecture (v2):
//!
//!   MiControl_Setup.exe   ← THE SHELL (static; hash stable across releases)
//!   MiControl_<ver>_payload.zip (+ .sig)   ← THE PAYLOAD (versioned, signed)
//!
//! The shell contains NO application payload. It installs itself under
//! `<INSTDIR>\setup\` and from there it performs EVERY privileged operation:
//! first install, update and uninstall. Because updates only replace the
//! payload files and never the shell itself, the shell never has to kill its
//! own process — the class of SCM/update races that plagued the old NSIS
//! flow (installer killing the very bridge service that launched it) is
//! structurally eliminated.

#![deny(rust_2018_idioms)]

mod ctx;
mod elev;
mod install;
mod payload;
mod shortcut;
mod sys;
mod uninstall;

use std::path::PathBuf;

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAIL: i32 = 1;

/// CLI options parsed once, shared by all commands.
#[derive(Debug, Default, Clone)]
pub struct Options {
    pub payload: Option<PathBuf>,
    pub url: Option<String>,
    pub dir: Option<PathBuf>,
    pub repo: Option<String>,
    pub silent: bool,
    pub no_relaunch: bool,
    /// Danger switch: apply a payload even without a .sig (dev only).
    pub allow_unsigned: bool,
    /// S65-004: version the caller expects the payload manifest to declare
    /// (e.g. from `MiControl_<ver>_payload.zip`). A disagreement aborts the
    /// update before anything is stopped.
    pub expect_version: Option<String>,
}

fn print_usage() {
    println!(
        "MiControl setup shell {version}

USAGE:
  micontrol_setup install   [--payload <zip>] [--url <url>] [--dir <dir>] [--silent]
      Elevates, copies this shell to <dir>\\setup\\, then applies the payload
      (downloads the latest signed payload when none is given).

  micontrol_setup update    --payload <zip> [--dir <dir>] [--no-relaunch] [--expect-version <v>]
      Applies a new payload over an existing installation (kills the app and
      the auxiliary services first, re-creates them afterwards).
      --expect-version refuses to apply a payload whose manifest version
      disagrees with <v> (S65-004 fail-closed guard).

  micontrol_setup uninstall [--dir <dir>] [--silent]
      Removes MiControl completely (relaunches itself from %TEMP% so the
      install directory can be fully deleted).

  micontrol_setup status    [--dir <dir>]
      Prints the resolved install directory and installed payload version.

  micontrol_setup help
      Shows this message.",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
}

fn run(args: &[String]) -> i32 {
    let mut it = args.iter();
    let cmd = match it.next() {
        Some(c) => c.as_str(),
        None => {
            print_usage();
            return EXIT_FAIL;
        }
    };

    let mut opts = Options::default();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--payload" => match it.next() {
                Some(v) => opts.payload = Some(PathBuf::from(v)),
                None => {
                    eprintln!("error: --payload requires a value");
                    return EXIT_FAIL;
                }
            },
            "--url" => match it.next() {
                Some(v) => opts.url = Some(v.clone()),
                None => {
                    eprintln!("error: --url requires a value");
                    return EXIT_FAIL;
                }
            },
            "--dir" => match it.next() {
                Some(v) => opts.dir = Some(PathBuf::from(v)),
                None => {
                    eprintln!("error: --dir requires a value");
                    return EXIT_FAIL;
                }
            },
            "--repo" => match it.next() {
                Some(v) => opts.repo = Some(v.clone()),
                None => {
                    eprintln!("error: --repo requires a value");
                    return EXIT_FAIL;
                }
            },
            "/S" | "--silent" => opts.silent = true,
            "--no-relaunch" => opts.no_relaunch = true,
            "--allow-unsigned" => opts.allow_unsigned = true,
            "--expect-version" => match it.next() {
                Some(v) => opts.expect_version = Some(v.clone()),
                None => {
                    eprintln!("error: --expect-version requires a value");
                    return EXIT_FAIL;
                }
            },
            other => {
                eprintln!("error: unknown option '{other}'");
                print_usage();
                return EXIT_FAIL;
            }
        }
    }

    match cmd {
        "install" => cmd_install(&opts),
        "apply-install" => cmd_apply_install(&opts),
        "update" => cmd_update(&opts),
        "uninstall" => cmd_uninstall(&opts),
        "uninstall-finish" => cmd_uninstall_finish(&opts),
        "status" => cmd_status(&opts),
        "bootstrap-shell" => cmd_bootstrap_shell(&opts),
        "help" | "--help" | "-h" | "/?" => {
            print_usage();
            EXIT_OK
        }
        other => {
            eprintln!("error: unknown command '{other}'");
            print_usage();
            EXIT_FAIL
        }
    }
}

/// `install`: first-time setup.
///
/// The downloaded shell runs from wherever the user saved it. It must NOT
/// perform privileged work from a location the user may delete afterwards —
/// so step 1 is to copy itself into `<INSTDIR>\setup\` and let THAT copy do
/// the work (and later every update/uninstall). The downloaded file becomes
/// disposable; the machine keeps one canonical shell with a stable hash.
fn cmd_install(opts: &Options) -> i32 {
    if let Err(e) = elev::ensure_elevated() {
        eprintln!("error: {e}");
        return EXIT_FAIL;
    }

    let dir = ctx::resolve_install_dir(opts.dir.as_deref(), true);
    println!("==> install directory: {}", dir.display());

    let shell_src = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("micontrol_setup"));
    let shell_dest = ctx::shell_path(&dir);

    // Refresh the installed copy of the shell (idempotent). Only restart the
    // work from the installed copy when we are NOT already that copy.
    if ctx::same_file(&shell_src, &shell_dest) {
        return install::apply_install(opts, &dir, true);
    }

    if let Some(parent) = shell_dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: cannot create {}: {e}", parent.display());
            return EXIT_FAIL;
        }
    }
    for attempt in 1..=5u32 {
        match std::fs::copy(&shell_src, &shell_dest) {
            Ok(_) => break,
            Err(e) if attempt < 5 => {
                // A previous shell instance may still hold the file briefly.
                let _ = std::fs::remove_file(&shell_dest);
                println!("==> shell copy retry {attempt}/5 ({e}) — waiting 1 s");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Err(e) => {
                eprintln!(
                    "error: cannot install shell to {}: {e}",
                    shell_dest.display()
                );
                return EXIT_FAIL;
            }
        }
    }
    println!("==> shell installed: {}", shell_dest.display());

    // Continue from the installed copy, detached, and let this (disposable)
    // process exit. The child performs the payload install.
    let mut args: Vec<String> = vec![
        "apply-install".to_string(),
        "--dir".to_string(),
        dir.to_string_lossy().into_owned(),
    ];
    if opts.silent {
        args.push("--silent".to_string());
    }
    if let Some(p) = &opts.payload {
        args.push("--payload".to_string());
        args.push(p.to_string_lossy().into_owned());
    }
    if let Some(u) = &opts.url {
        args.push("--url".to_string());
        args.push(u.clone());
    }
    if let Some(r) = &opts.repo {
        args.push("--repo".to_string());
        args.push(r.clone());
    }

    match sys::spawn_detached(&shell_dest, &args) {
        Ok(()) => {
            println!("==> continuing from the installed shell.");
            EXIT_OK
        }
        Err(e) => {
            eprintln!("error: cannot relaunch installed shell: {e}");
            EXIT_FAIL
        }
    }
}

/// `apply-install` (internal): runs from `<INSTDIR>\setup\` with elevation and
/// performs the full first install: WebView2, payload (download/verify),
/// extraction, install points, shortcuts + uninstall registration, launch.
fn cmd_apply_install(opts: &Options) -> i32 {
    if let Err(e) = elev::ensure_elevated() {
        eprintln!("error: {e}");
        return EXIT_FAIL;
    }
    let dir = ctx::resolve_install_dir(opts.dir.as_deref(), true);
    install::apply_install(opts, &dir, true)
}

/// `update`: apply a new payload over an existing installation.
///
/// Called by the MiControlBridge SYSTEM service (or manually by an admin).
/// The shell lives under `<INSTDIR>\setup\` and is never part of the payload,
/// so nothing here can terminate or replace the running shell process.
fn cmd_update(opts: &Options) -> i32 {
    if let Err(e) = elev::ensure_elevated() {
        eprintln!("error: {e}");
        return EXIT_FAIL;
    }
    let dir = ctx::resolve_install_dir(opts.dir.as_deref(), false);
    if opts.payload.is_none() {
        eprintln!("error: update requires --payload <zip>");
        return EXIT_FAIL;
    }
    println!("==> update directory: {}", dir.display());
    if let Some(p) = &opts.payload {
        println!("==> payload         : {}", p.display());
    }
    install::apply_install(opts, &dir, false)
}

/// `uninstall`: copy self to %TEMP%, then remove everything from there (the
/// shell binary itself lives inside the directory being deleted).
fn cmd_uninstall(opts: &Options) -> i32 {
    if let Err(e) = elev::ensure_elevated() {
        eprintln!("error: {e}");
        return EXIT_FAIL;
    }
    let dir = ctx::resolve_install_dir(opts.dir.as_deref(), false);
    let self_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("micontrol_setup"));
    let temp_exe = std::env::temp_dir().join("MiControlSetup_uninst.exe");

    if ctx::is_under(&self_exe, &dir) && !ctx::same_file(&self_exe, &temp_exe) {
        let _ = std::fs::remove_file(&temp_exe);
        if let Err(e) = std::fs::copy(&self_exe, &temp_exe) {
            eprintln!("error: cannot stage uninstaller in %TEMP%: {e}");
            return EXIT_FAIL;
        }
        let args = [
            "uninstall-finish".to_string(),
            "--dir".to_string(),
            dir.to_string_lossy().into_owned(),
        ];
        if let Err(e) = sys::spawn_detached(&temp_exe, &args) {
            eprintln!("error: cannot launch staged uninstaller: {e}");
            return EXIT_FAIL;
        }
        println!("==> staged uninstaller launched from %TEMP%.");
        return EXIT_OK;
    }
    uninstall::run(&dir, true)
}

/// `uninstall-finish` (internal): runs from %TEMP% and actually deletes the
/// installation.
fn cmd_uninstall_finish(opts: &Options) -> i32 {
    // Give the staging process time to exit and release its handles.
    std::thread::sleep(std::time::Duration::from_millis(800));
    if let Err(e) = elev::ensure_elevated() {
        eprintln!("error: {e}");
        return EXIT_FAIL;
    }
    let dir = ctx::resolve_install_dir(opts.dir.as_deref(), false);
    let code = uninstall::run(&dir, true);

    // Best-effort self-cleanup of the staged copy.
    let self_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::new());
    let _ = std::fs::remove_file(&self_exe);
    code
}

fn cmd_status(opts: &Options) -> i32 {
    let dir = ctx::resolve_install_dir(opts.dir.as_deref(), false);
    println!("install dir : {}", dir.display());
    println!("shell       : {}", ctx::shell_path(&dir).display());
    println!("shell ver   : {}", env!("CARGO_PKG_VERSION"));
    match install::read_payload_version(&dir) {
        Some(v) => println!("payload ver : {v}"),
        None => println!("payload ver : (not installed)"),
    }
    EXIT_OK
}

/// `bootstrap` (internal): download + verify the static shell binary and
/// place it into `<dir>\setup\` for legacy NSIS installations that predate
/// the decoupled installer. The bridge calls this when the shell is missing
/// but a payload update is in flight (S65-002).
///
/// Flow:
///   1. Download `MiControl_Setup.exe` from the latest GitHub release.
///   2. Verify size sanity and Authenticode signature (signtool).
///   3. Write to `<dir>\setup\micontrol_setup.exe`.
///   4. Launch the shell with `update --payload <zip>` (detached).
///
/// The shell is a static artifact with a stable hash — it never replaces
/// itself, so the bootstrap is a one-time migration that survives future
/// payload updates without further downloads.
fn cmd_bootstrap_shell(opts: &Options) -> i32 {
    let dir = crate::ctx::resolve_install_dir(opts.dir.as_deref(), false);
    let shell_path = crate::ctx::shell_path(&dir);

    // Fast path: already installed.
    if shell_path.is_file() {
        let len = std::fs::metadata(&shell_path).map(|m| m.len()).unwrap_or(0);
        if len >= 500_000 {
            println!("==> shell already installed: {}", shell_path.display());
            return EXIT_OK;
        }
        println!("==> shell present but only {len} bytes — re-downloading.");
    }

    let repo = opts.repo.as_deref().unwrap_or(crate::ctx::DEFAULT_REPO);
    let url = format!("https://github.com/{repo}/releases/latest/download/MiControl_Setup.exe");
    println!("==> downloading shell: {url}");

    let cache = crate::ctx::local_appdata_dir().join("setup");
    if let Err(e) = std::fs::create_dir_all(&cache) {
        eprintln!("error: cannot create {}: {e}", cache.display());
        return EXIT_FAIL;
    }
    let dest = cache.join("MiControl_Setup.exe");
    match crate::payload::download_public(&url, &dest) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("error: download failed: {e}");
            return EXIT_FAIL;
        }
    }

    let len = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    if !(500_000..=16 * 1024 * 1024).contains(&len) {
        eprintln!(
            "error: downloaded shell is {len} bytes (expected 500000..16777216) — rejecting."
        );
        let _ = std::fs::remove_file(&dest);
        return EXIT_FAIL;
    }

    // Authenticode verify (best-effort; signtool may not be available in CI).
    let sig_ok = verify_authenticode(&dest);
    if !sig_ok {
        eprintln!(
            "WARNING: Authenticode signature could not be verified for the downloaded \
             shell (signtool unavailable or signature missing). Proceeding with size \
             check only — the file was downloaded from the trusted GitHub release."
        );
    } else {
        println!("==> Authenticode signature verified.");
    }

    // Write into the install directory.
    if let Some(parent) = shell_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: cannot create {}: {e}", parent.display());
            return EXIT_FAIL;
        }
    }
    for attempt in 1..=5u32 {
        match std::fs::copy(&dest, &shell_path) {
            Ok(_) => break,
            Err(e) if attempt < 5 => {
                let _ = std::fs::remove_file(&shell_path);
                println!("==> shell copy retry {attempt}/5 ({e}) — waiting 1 s");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Err(e) => {
                eprintln!(
                    "error: cannot install shell to {}: {e}",
                    shell_path.display()
                );
                return EXIT_FAIL;
            }
        }
    }
    let _ = std::fs::remove_file(&dest);
    println!("==> shell installed: {}", shell_path.display());

    // If a payload was specified, launch the shell to apply it.
    if let Some(p) = &opts.payload {
        let args = [
            "update".to_string(),
            "--payload".to_string(),
            p.to_string_lossy().into_owned(),
            "--dir".to_string(),
            dir.to_string_lossy().into_owned(),
        ];
        match crate::sys::spawn_detached(&shell_path, &args) {
            Ok(()) => println!("==> shell launched to apply payload."),
            Err(e) => {
                eprintln!("error: cannot launch shell for payload update: {e}");
                return EXIT_FAIL;
            }
        }
    }

    EXIT_OK
}

/// Best-effort Authenticode verification via `signtool verify`.
/// Returns `true` only when the signature is positively verified.
fn verify_authenticode(path: &std::path::Path) -> bool {
    use crate::sys::run_capture;
    let signtool = [
        r"C:\Program Files (x86)\Windows Kits\10\bin\x86\signtool.exe",
        r"C:\Program Files\Windows Kits\10\bin\x86\signtool.exe",
    ]
    .iter()
    .find(|p| std::path::Path::new(p).is_file())
    .copied()
    .unwrap_or("signtool.exe");
    let path_str = path.display().to_string();
    let (code, out) = run_capture(signtool, &["verify", "/pa", "/v", path_str.as_str()]);
    if code == 0 {
        println!("    signtool verify: OK");
        true
    } else {
        eprintln!("    signtool verify returned {code}: {out}");
        false
    }
}
