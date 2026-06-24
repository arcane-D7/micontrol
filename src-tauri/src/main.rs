//! MiControl binary entry point.
//!
//! Detects elevated helper mode (`--elevated`) and dispatches to
//! the privileged command runner, or starts the full Tauri desktop application.

#![windows_subsystem = "windows"]

fn main() {
    // If launched as the privileged helper by the scheduled task, execute the
    // requested hardware command and exit — no Tauri window is opened.
    if std::env::args().any(|a| a == "--elevated") {
        micontrol_lib::elevated::run(); // -> !
    }
    micontrol_lib::run();
}
