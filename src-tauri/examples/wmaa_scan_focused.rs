//! Focused WMAA scanner — scans only promising FUN2 groups.
//!
//! Instead of scanning all 65536 combinations, this scans:
//! - Known groups (0x0800, 0x0A00, 0x0C00, 0x1000) with all FUN3 values
//! - Nearby groups (0x0900, 0x0B00, 0x0D00, 0x0E00, 0x0F00, 0x1100, 0x1200, etc.)
//! - All groups with high nibble patterns (0x0n00, 0x1n00)
//!
//! Usage (must run as admin):
//!   cargo run --example wmaa_scan_focused

#![cfg(windows)]

use micontrol_lib::hw::wmi_ec;

fn main() {
    println!("=== WMAA Focused Scanner ===");
    println!("Scanning promising FUN2 groups × all FUN3 values\n");

    // Known working groups + nearby candidates
    let groups: &[u16] = &[
        // Known working
        0x0800, 0x0A00, 0x0C00, 0x1000, // Nearby (±1, ±2 from known)
        0x0700, 0x0900, 0x0B00, 0x0D00, 0x0E00, 0x0F00, 0x1100, 0x1200,
        // Low range (might have basic EC functions)
        0x0100, 0x0200, 0x0300, 0x0400, 0x0500, 0x0600,
        // Higher range (might have extended sensors)
        0x1400, 0x1800, 0x1C00, 0x2000, // Non-aligned (sub-groups within known ranges)
        0x0801, 0x0A01, 0x0C01, 0x1001, 0x0802, 0x0A02, 0x0C02, 0x1002,
        // Special: try 0x0000 (might be a "get all" command)
        0x0000,
    ];

    let mut hits: Vec<(u16, u16, wmi_ec::WmaaResponse)> = Vec::new();

    for &fun2 in groups {
        print!("[scanning] FUN2=0x{fun2:04X} ... ");
        let mut group_hits = 0;
        for fun3 in 0u16..=0xFF {
            match wmi_ec::wmi_read(fun2, fun3) {
                Ok(resp) => {
                    if resp.is_success() {
                        println!("\n  [HIT] FUN3=0x{fun3:02X} → SGER=0x{:04X} FUTR=0x{:04X} FRD0=0x{:04X} FRD1=0x{:08X} FRD2=0x{:08X} FRD3=0x{:08X}",
                            resp.sger, resp.futr, resp.frd0, resp.frd1, resp.frd2, resp.frd3);
                        hits.push((fun2, fun3, resp));
                        group_hits += 1;
                    }
                }
                Err(_) => {}
            }
        }
        if group_hits == 0 {
            println!("no hits");
        } else {
            println!("  → {group_hits} hits in this group");
        }
    }

    println!("\n=== Scan Complete ===");
    println!("Total hits: {}", hits.len());

    // Group by FUN2
    println!("\n--- Summary by FUN2 ---");
    let mut fun2_groups: std::collections::BTreeMap<u16, Vec<(u16, &wmi_ec::WmaaResponse)>> =
        std::collections::BTreeMap::new();
    for (fun2, fun3, resp) in &hits {
        fun2_groups.entry(*fun2).or_default().push((*fun3, resp));
    }
    for (fun2, entries) in &fun2_groups {
        let known = matches!(*fun2, 0x0800 | 0x0A00 | 0x0C00 | 0x1000);
        let tag = if known { "known" } else { "*** NEW ***" };
        println!("\nFUN2=0x{fun2:04X} ({tag}) — {} hits:", entries.len());
        for (fun3, resp) in entries {
            println!(
                "  FUN3=0x{fun3:02X} → FRD0=0x{:04X} FRD1=0x{:08X} FRD2=0x{:08X} FRD3=0x{:08X}",
                resp.frd0, resp.frd1, resp.frd2, resp.frd3
            );
        }
    }

    // Write results
    let results: String = hits
        .iter()
        .map(|(f2, f3, r)| {
            format!(
                "FUN2=0x{f2:04X} FUN3=0x{f3:02X} SGER=0x{:04X} FUTR=0x{:04X} FRD0=0x{:04X} FRD1=0x{:08X} FRD2=0x{:08X} FRD3=0x{:08X}",
                r.sger, r.futr, r.frd0, r.frd1, r.frd2, r.frd3
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write("wmaa_scan_focused_results.txt", results);
    println!("\nResults saved to wmaa_scan_focused_results.txt");
}
