//! Windows Defender Firewall rules for MiControl's own inbound listeners.
//!
//! Why this exists (audit 2026-11-07, `uac-firewall.md`)
//! ─────────────────────────────────────────────────────
//! `micontrol.exe` opens `0.0.0.0:9223` (MCP bridge WebSocket) and, when the
//! user turns on device-to-device transfer, `0.0.0.0:53317` TCP+UDP
//! (LocalSend receiver). With no matching rule, Windows raises the
//! "Allow MiControl to communicate on these networks: Private / Public"
//! dialog — once per *program path* and *network profile*. The dialog-created
//! rules are `Defer=User`, `Public`-only, and are re-created for every new
//! build path (`target\debug\deps\micontrol_lib-<hash>.exe`), which is why the
//! prompt kept coming back.
//!
//! The fix is deterministic rules, applied elevated at install time:
//!
//! * allow TCP 9223 and TCP/UDP 53317 for the installed `micontrol.exe` on
//!   `private,domain` only (no `edge=yes` — no NAT-traversal exception);
//! * an explicit **block** rule for `public`, which is what actually silences
//!   the dialog: Windows only asks when *no* rule matches the request.
//!
//! The MCP bridge itself now binds loopback only (`util::mcp_config`), so the
//! 9223 rule is defence in depth for older builds and for the LocalSend port.
//!
//! Everything here is best-effort by design: a firewall failure must never
//! roll back a working installation — the app still runs, it just may prompt
//! again. Callers decide whether to log a warning or fail.

use std::path::Path;

/// Installed main binary the rules are scoped to.
pub const MAIN_EXE: &str = "micontrol.exe";

/// Rule names. Stable strings: `apply` deletes by program first, so a rename
/// can never leave a duplicate behind.
pub const RULE_MCP: &str = "MiControl MCP (TCP 9223)";
pub const RULE_LOCALSEND_TCP: &str = "MiControl LocalSend (TCP 53317)";
pub const RULE_LOCALSEND_UDP: &str = "MiControl LocalSend (UDP 53317)";
pub const RULE_BLOCK_PUBLIC: &str = "MiControl (block public)";

/// Ports the app listens on (kept in sync with the audit + LocalSend).
pub const PORT_MCP: u16 = 9223;
pub const PORT_LOCALSEND: u16 = 53317;
/// KDE Connect discovery port. The discovery socket binds this fixed port
/// since the ephemeral-bind fix (see hw::kde_connect) — an ephemeral port
/// could never be covered by a deterministic rule and popped the dialog.
pub const PORT_KDE_UDP: u16 = 1716;
pub const RULE_KDE_UDP: &str = "MiControl KDE Connect (UDP 1716)";

/// `program=` token for `netsh`.
///
/// The value must NOT contain quote characters: netsh parses its own argv and
/// rejects a value whose literal text is wrapped in `"` ("A specified value is
/// not valid", verified against netsh on Windows 11 26100). Spaces are handled
/// by the C runtime that builds the child command line — `std::process::Command`
/// quotes the whole `program=...` token, the CRT strips that quoting, and netsh
/// receives `program=C:\Program Files\MiControl\micontrol.exe` as one argument,
/// which is exactly what a quoted `program="..."` produces from cmd.exe.
pub fn program_arg(exe: &str) -> String {
    format!("program={exe}")
}

/// The exact `netsh advfirewall` argument vectors, in execution order.
///
/// Step 1 deletes **every** rule that matches the program — that is what
/// removes the `Defer=User` rules created by the firewall dialog (they are
/// named after the binary description, not by us) and makes the whole
/// sequence idempotent: re-running it never accumulates duplicates.
pub fn netsh_steps(exe: &str) -> Vec<Vec<String>> {
    let program = program_arg(exe);
    let s = |v: &str| v.to_string();
    vec![
        vec![
            s("advfirewall"),
            s("firewall"),
            s("delete"),
            s("rule"),
            s("name=all"),
            program.clone(),
        ],
        vec![
            s("advfirewall"),
            s("firewall"),
            s("add"),
            s("rule"),
            format!("name={RULE_MCP}"),
            s("dir=in"),
            s("action=allow"),
            program.clone(),
            s("protocol=TCP"),
            format!("localport={PORT_MCP}"),
            s("profile=private,domain"),
            s("enable=yes"),
            s("edge=no"),
        ],
        vec![
            s("advfirewall"),
            s("firewall"),
            s("add"),
            s("rule"),
            format!("name={RULE_LOCALSEND_TCP}"),
            s("dir=in"),
            s("action=allow"),
            program.clone(),
            s("protocol=TCP"),
            format!("localport={PORT_LOCALSEND}"),
            s("profile=private,domain"),
            s("enable=yes"),
            s("edge=no"),
        ],
        vec![
            s("advfirewall"),
            s("firewall"),
            s("add"),
            s("rule"),
            format!("name={RULE_LOCALSEND_UDP}"),
            s("dir=in"),
            s("action=allow"),
            program.clone(),
            s("protocol=UDP"),
            format!("localport={PORT_LOCALSEND}"),
            s("profile=private,domain"),
            s("enable=yes"),
            s("edge=no"),
        ],
        vec![
            s("advfirewall"),
            s("firewall"),
            s("add"),
            s("rule"),
            format!("name={RULE_KDE_UDP}"),
            s("dir=in"),
            s("action=allow"),
            program.clone(),
            s("protocol=UDP"),
            format!("localport={PORT_KDE_UDP}"),
            s("profile=private,domain"),
            s("enable=yes"),
            s("edge=no"),
        ],
        vec![
            s("advfirewall"),
            s("firewall"),
            s("add"),
            s("rule"),
            format!("name={RULE_BLOCK_PUBLIC}"),
            s("dir=in"),
            s("action=block"),
            program,
            s("protocol=any"),
            s("profile=public"),
            s("enable=yes"),
        ],
    ]
}

/// Run one `netsh` invocation, returning `(exit_code, combined_output)`.
fn run_netsh(args: &[String]) -> (i32, String) {
    use std::process::Command;

    let mut cmd = Command::new("netsh.exe");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
    }
    match cmd.output() {
        Ok(out) => {
            let code = out.status.code().unwrap_or(-1);
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                if !text.trim().is_empty() {
                    text.push(' ');
                }
                text.push_str(&err);
            }
            (code, text)
        }
        Err(e) => (-1, format!("failed to spawn netsh.exe: {e}")),
    }
}

/// True when `exe` looks like a cargo build artifact (`...\target\<profile>\...`).
///
/// The rules must only be scoped to the *installed* binary. In dev the bridge
/// can run from `target\debug`, and creating rules for those paths is exactly
/// how the machine accumulated 28 stale `micontrol_lib-<hash>.exe` entries.
pub fn is_dev_binary(exe: &Path) -> bool {
    exe.to_string_lossy()
        .to_ascii_lowercase()
        .contains("\\target\\")
}

/// Apply the deterministic rule set for `install_dir\micontrol.exe`.
///
/// Must run elevated (the NSIS installer and the SYSTEM bridge both are).
/// Returns the first failing step as `Err`; the caller decides whether that is
/// fatal (it never is for the app: the rules only suppress a dialog).
pub fn apply(install_dir: &Path) -> Result<(), String> {
    let exe = install_dir.join(MAIN_EXE);
    if !exe.is_file() {
        return Err(format!(
            "{} not found — firewall rules not applied",
            exe.display()
        ));
    }
    if is_dev_binary(&exe) {
        return Err(format!(
            "{} is a build artifact, not an installed binary — firewall rules not applied",
            exe.display()
        ));
    }
    let exe_str = exe.display().to_string();
    for args in netsh_steps(&exe_str) {
        let (code, out) = run_netsh(&args);
        if code != 0 {
            return Err(format!(
                "netsh {} failed (exit {code}): {}",
                args.join(" "),
                out.trim()
            ));
        }
    }
    log::info!(
        "[firewall] rules applied for {} (allow private,domain 9223/53317/1716-udp; block public)",
        exe.display()
    );
    Ok(())
}

/// Remove every rule scoped to `install_dir\micontrol.exe` (uninstall path).
pub fn remove(install_dir: &Path) -> Result<(), String> {
    let exe = install_dir.join(MAIN_EXE);
    let args = vec![
        "advfirewall".to_string(),
        "firewall".to_string(),
        "delete".to_string(),
        "rule".to_string(),
        "name=all".to_string(),
        program_arg(&exe.display().to_string()),
    ];
    let (code, out) = run_netsh(&args);
    if code != 0 {
        return Err(format!(
            "netsh {} failed (exit {code}): {}",
            args.join(" "),
            out.trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Program Files\MiControl\micontrol.exe";

    fn find<'a>(steps: &'a [Vec<String>], needle: &str) -> &'a Vec<String> {
        steps
            .iter()
            .find(|s| s.iter().any(|a| a == needle))
            .unwrap_or_else(|| panic!("no step containing {needle}"))
    }

    #[test]
    fn first_step_deletes_every_rule_for_the_program() {
        let steps = netsh_steps(EXE);
        let first = &steps[0];
        assert_eq!(
            first,
            &vec![
                "advfirewall".to_string(),
                "firewall".to_string(),
                "delete".to_string(),
                "rule".to_string(),
                "name=all".to_string(),
                program_arg(EXE),
            ],
            "the dialog-created rules are only removable by program, not by name"
        );
    }

    #[test]
    fn program_token_has_no_embedded_quotes() {
        // netsh parses its own argv: a value whose literal text is wrapped in
        // `"` is rejected ("A specified value is not valid"). The quotes must
        // come from the CRT when the child command line is built — passing the
        // whole `program=...` token unquoted is what makes
        // `program="C:\Program Files\..."` out of it.
        assert_eq!(
            program_arg(EXE),
            r"program=C:\Program Files\MiControl\micontrol.exe"
        );
        assert!(!program_arg(EXE).contains('"'));
    }

    #[test]
    fn no_argument_carries_a_literal_quote_character() {
        // Regression guard for the exact defect found while validating this
        // module: emitting `program="..."` makes netsh fail with
        // "A specified value is not valid" (rules silently not applied).
        for step in netsh_steps(EXE) {
            for arg in &step {
                assert!(
                    !arg.contains('"'),
                    "argument {arg:?} contains a literal quote — netsh would reject it"
                );
            }
        }
    }

    #[test]
    fn allow_rules_are_private_domain_only_and_never_edge_traversal() {
        let steps = netsh_steps(EXE);
        for name in [
            RULE_MCP,
            RULE_LOCALSEND_TCP,
            RULE_LOCALSEND_UDP,
            RULE_KDE_UDP,
        ] {
            let step = find(&steps, &format!("name={name}"));
            assert!(step.contains(&"action=allow".to_string()), "{name}");
            assert!(step.contains(&"dir=in".to_string()), "{name}");
            assert!(
                step.contains(&"profile=private,domain".to_string()),
                "{name} must not be allowed on public networks"
            );
            assert!(
                step.contains(&"edge=no".to_string()),
                "{name} must not request NAT-traversal"
            );
            assert!(step.contains(&"enable=yes".to_string()), "{name}");
            assert!(step.contains(&program_arg(EXE)), "{name}");
        }
    }

    #[test]
    fn ports_and_protocols_match_the_listeners() {
        let steps = netsh_steps(EXE);
        let mcp = find(&steps, &format!("name={RULE_MCP}"));
        assert!(mcp.contains(&"protocol=TCP".to_string()));
        assert!(mcp.contains(&format!("localport={PORT_MCP}")));

        let tcp = find(&steps, &format!("name={RULE_LOCALSEND_TCP}"));
        assert!(tcp.contains(&"protocol=TCP".to_string()));
        assert!(tcp.contains(&format!("localport={PORT_LOCALSEND}")));

        let udp = find(&steps, &format!("name={RULE_LOCALSEND_UDP}"));
        assert!(udp.contains(&"protocol=UDP".to_string()));
        assert!(udp.contains(&format!("localport={PORT_LOCALSEND}")));

        // KDE discovery must be a FIXED port (1716): an ephemeral bind can
        // never be covered by a deterministic rule — that moving port was the
        // recurring firewall dialog.
        let kde = find(&steps, &format!("name={RULE_KDE_UDP}"));
        assert!(kde.contains(&"protocol=UDP".to_string()));
        assert!(kde.contains(&format!("localport={PORT_KDE_UDP}")));
    }

    #[test]
    fn public_profile_is_blocked_for_every_protocol() {
        // This is the rule that suppresses the dialog: Windows only asks when
        // no rule matches, and a block rule matches.
        let steps = netsh_steps(EXE);
        let block = find(&steps, &format!("name={RULE_BLOCK_PUBLIC}"));
        assert!(block.contains(&"action=block".to_string()));
        assert!(block.contains(&"profile=public".to_string()));
        assert!(block.contains(&"protocol=any".to_string()));
        assert!(block.contains(&"dir=in".to_string()));
        assert!(block.contains(&program_arg(EXE)));
    }

    #[test]
    fn every_step_starts_with_the_advfirewall_prefix() {
        for step in netsh_steps(EXE) {
            assert_eq!(&step[0], "advfirewall");
            assert_eq!(&step[1], "firewall");
        }
    }

    #[test]
    fn apply_refuses_a_missing_binary_without_touching_the_firewall() {
        let dir = std::env::temp_dir().join(format!("mc-fw-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let err = apply(&dir).expect_err("no micontrol.exe → must not run netsh");
        assert!(err.contains("not found"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dev_build_artifacts_are_never_given_firewall_rules() {
        // Guards against re-creating the 28 stale target\debug\deps rules the
        // audit found: only an installed binary may get rules.
        assert!(is_dev_binary(Path::new(
            r"C:\Users\me\Documents\Projects\miPC\micontrol\src-tauri\target\debug\micontrol.exe"
        )));
        assert!(is_dev_binary(Path::new(
            r"C:\Users\me\proj\target\debug\deps\micontrol_lib-1a2b3c.exe"
        )));
        assert!(!is_dev_binary(Path::new(
            r"C:\Program Files\MiControl\micontrol.exe"
        )));

        let dir =
            std::env::temp_dir().join(format!("mc-fw-dev-{}\\target\\debug", std::process::id()));
        let _ = std::fs::remove_dir_all(dir.parent().and_then(|p| p.parent()).unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MAIN_EXE), b"MZ").unwrap();
        let err = apply(&dir).expect_err("a build artifact must be skipped");
        assert!(err.contains("build artifact"), "got: {err}");
        let _ = std::fs::remove_dir_all(dir.parent().and_then(|p| p.parent()).unwrap());
    }
}
