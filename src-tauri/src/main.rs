//! MiControl binary entry point.
//!
//! Detects elevated helper mode (`--elevated`) and dispatches to
//! the privileged command runner, or starts the full Tauri desktop application.

#![windows_subsystem = "windows"]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // If launched as the privileged helper by the scheduled task, execute all
    // requested hardware commands and exit — no Tauri window is opened.
    if args.iter().any(|a| a == "--elevated") {
        micontrol_lib::elevated::run();
        return;
    }

    // S26-004: Manual key rotation CLI handler.
    if args.iter().any(|a| a == "--rotate-key") {
        match micontrol_lib::util::auth::rotate_key() {
            Ok(()) => {
                println!("HMAC key rotated successfully.");
                return;
            }
            Err(e) => {
                eprintln!("Failed to rotate HMAC key: {e}");
                std::process::exit(1);
            }
        }
    }

    micontrol_lib::run();
}
