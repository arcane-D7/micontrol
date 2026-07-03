//! WMAA FUN2×FUN3 space scanner.
//!
//! Scans all FUN2 (0x0000–0xFFFF, step 0x0100) × FUN3 (0x00–0xFF)
//! combinations via WMI MiInterface read calls, looking for responses
//! with SGER=0x8000 (success). This discovers undocumented WMAA commands
//! that may provide access to keyboard backlight, fan RPM, battery
//! current/voltage, and Smart Mode.
//!
//! Usage (must run as admin):
//!   cargo run --example wmaa_scan

#![cfg(windows)]

use micontrol_lib::hw::wmi_ec;

fn main() {
    println!("=== WMAA FUN2×FUN3 Space Scanner ===");
    println!("Scanning all FUN2 (0x0000-0xFF00, step 0x0100) × FUN3 (0x00-0xFF)");
    println!("Looking for SGER=0x8000 (success) responses\n");

    // Known working FUN2 groups
    let known_groups: &[u16] = &[0x0800, 0x0A00, 0x0C00, 0x1000];

    let mut hits: Vec<(u16, u16, wmi_ec::WmaaResponse)> = Vec::new();
    let mut errors: Vec<(u16, u16, String)> = Vec::new();

    // Scan FUN2 from 0x0000 to 0xFF00, step 0x0100
    for fun2_idx in 0u16..=0xFF {
        let fun2 = fun2_idx << 8; // 0x0000, 0x0100, 0x0200, ..., 0xFF00

        // Also try non-aligned values for known groups
        let is_known = known_groups.contains(&fun2);

        for fun3 in 0u16..=0xFF {
            match wmi_ec::wmi_read(fun2, fun3) {
                Ok(resp) => {
                    if resp.is_success() {
                        let is_new = !is_known;
                        let tag = if is_new { "NEW" } else { "known" };
                        println!(
                            "[HIT {tag}] FUN2=0x{fun2:04X} FUN3=0x{fun3:02X} → \
                             SGER=0x{:04X} FUTR=0x{:04X} FRD0=0x{:04X} FRD1=0x{:08X} FRD2=0x{:08X} FRD3=0x{:08X}",
                            resp.sger, resp.futr, resp.frd0, resp.frd1, resp.frd2, resp.frd3
                        );
                        hits.push((fun2, fun3, resp));
                    }
                }
                Err(e) => {
                    // Only log errors for known groups (to avoid noise)
                    if is_known {
                        let msg = e.to_string();
                        if !msg.contains("Invalid parameter") && !msg.contains("Invalid object") {
                            errors.push((fun2, fun3, msg));
                        }
                    }
                }
            }
        }

        // Progress indicator every 16 groups
        if fun2_idx % 16 == 0 && fun2_idx > 0 {
            println!(
                "[progress] {}/256 groups scanned, {} hits so far",
                fun2_idx,
                hits.len()
            );
        }
    }

    println!("\n=== Scan Complete ===");
    println!("Total hits (SGER=0x8000): {}", hits.len());
    println!("Total errors logged: {}", errors.len());

    // Print summary grouped by FUN2
    println!("\n--- Hits by FUN2 group ---");
    let mut fun2_groups: std::collections::BTreeMap<u16, Vec<(u16, &wmi_ec::WmaaResponse)>> =
        std::collections::BTreeMap::new();
    for (fun2, fun3, resp) in &hits {
        fun2_groups.entry(*fun2).or_default().push((*fun3, resp));
    }
    for (fun2, entries) in &fun2_groups {
        let tag = if known_groups.contains(fun2) {
            "known"
        } else {
            "*** NEW ***"
        };
        println!("\nFUN2=0x{fun2:04X} ({tag}) — {} hits:", entries.len());
        for (fun3, resp) in entries {
            println!(
                "  FUN3=0x{fun3:02X} → FRD0=0x{:04X} FRD1=0x{:08X} FRD2=0x{:08X} FRD3=0x{:08X}",
                resp.frd0, resp.frd1, resp.frd2, resp.frd3
            );
        }
    }

    // Also scan with FUN1=0xFB00 (write) for groups that had read hits,
    // using FUN4=0 to see if write commands also succeed
    println!("\n--- Write probe (FUN1=0xFB00, FUN4=0) for hit groups ---");
    for (fun2, fun3, _) in &hits {
        match wmi_ec::wmi_write(*fun2, *fun3, 0) {
            Ok(resp) => {
                if resp.is_success() {
                    println!(
                        "[WRITE HIT] FUN2=0x{fun2:04X} FUN3=0x{fun3:02X} → SGER=0x{:04X}",
                        resp.sger
                    );
                }
            }
            Err(_) => {} // Ignore write errors
        }
    }

    // Save results to file
    let results = format!(
        "WMAA Scan Results\n\
         Total hits: {}\n\
         \n\
         {:?}",
        hits.len(),
        hits.iter()
            .map(|(f2, f3, r)| format!(
                "FUN2=0x{f2:04X} FUN3=0x{f3:02X} SGER=0x{:04X} FRD0=0x{:04X} FRD1=0x{:08X}",
                r.sger, r.frd0, r.frd1
            ))
            .collect::<Vec<_>>()
    );
    let _ = std::fs::write("wmaa_scan_results.txt", results);
    println!("\nResults saved to wmaa_scan_results.txt");
}
