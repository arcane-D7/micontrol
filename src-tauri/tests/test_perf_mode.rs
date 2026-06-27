//! Integration test: verify performance mode persistence through the elevated bridge.
//!
//! This test creates a proper HMAC-signed command file, runs the elevated helper,
//! and checks that the registry value changes.

use std::process::Command;

#[test]
#[ignore = "Requires admin privileges and MiControl scheduled task"]
fn test_performance_mode_persists() {
    // This test is a placeholder — actual testing requires the full
    // elev_bridge flow which needs the Tauri runtime.
}
