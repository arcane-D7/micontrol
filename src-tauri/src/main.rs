#![windows_subsystem = "windows"]

fn main() {
    // If launched as the privileged helper by the scheduled task, execute the
    // requested hardware command and exit — no Tauri window is opened.
    if std::env::args().any(|a| a == "--elevated") {
        micontrol_lib::elevated::run(); // -> !
    }
    micontrol_lib::run();
}
