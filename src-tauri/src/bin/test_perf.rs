//! Manual test: simulate the elevated bridge flow for various hardware commands.
//!
//! Usage: cargo run --bin test_perf -- [command] [args]
//! Examples:
//!   cargo run --bin test_perf -- set_performance_mode balance
//!   cargo run --bin test_perf -- set_charging_threshold 80
//!   cargo run --bin test_perf -- set_brightness 50

use hmac::{Hmac, Mac};
use rand::RngCore;
use serde_json::{json, Value};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: test_perf <command> [args...]");
        eprintln!("  set_performance_mode <mode>");
        eprintln!("  set_charging_threshold <threshold>");
        eprintln!("  set_brightness <level>");
        std::process::exit(1);
    }

    let cmd = &args[0];
    let cmd_args = match cmd.as_str() {
        "set_performance_mode" => json!({ "mode": args.get(1).unwrap_or(&"balance".to_string()) }),
        "set_charging_threshold" => {
            json!({ "threshold": args.get(1).unwrap_or(&"80".to_string()).parse::<u8>().unwrap_or(80) })
        }
        "set_brightness" => {
            json!({ "level": args.get(1).unwrap_or(&"50".to_string()).parse::<u8>().unwrap_or(50) })
        }
        _ => {
            eprintln!("Unknown command: {cmd}");
            std::process::exit(1);
        }
    };

    // Read the HMAC key
    let key_path = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    let key_path = std::path::Path::new(&key_path)
        .join("MiControl")
        .join("elev_key.bin");
    let key = std::fs::read(&key_path).expect("Cannot read key file");

    // Create a command payload
    let request_id = format!(
        "test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    let nonce = {
        let mut buf = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut buf);
        buf.iter().map(|b| format!("{:02x}", b)).collect::<String>()
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let mut payload = json!({
        "protocol_version": 2,
        "request_id": request_id,
        "created_at_ms": now_ms,
        "nonce": nonce,
        "caller_pid": std::process::id(),
        "cmd": cmd,
        "args": cmd_args,
    });

    // Sign the payload (same as auth::sign_payload)
    let body = payload.to_string();
    let mut mac = HmacSha256::new_from_slice(&key).unwrap();
    mac.update(body.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());
    payload["hmac"] = Value::String(signature);

    println!("Command: {cmd}");
    println!("Args: {}", serde_json::to_string(&cmd_args).unwrap());

    // Write the command file
    let cmd_path = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    let cmd_path = std::path::Path::new(&cmd_path)
        .join("MiControl")
        .join(format!("elev_cmd_{request_id}.json"));
    std::fs::write(&cmd_path, payload.to_string()).expect("Cannot write command file");

    // Run the scheduled task
    let output = std::process::Command::new("schtasks")
        .args(["/Run", "/TN", "MiControlElevated"])
        .output()
        .expect("Cannot run schtasks");
    println!(
        "schtasks: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );

    // Wait for the result
    let result_path = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    let result_path = std::path::Path::new(&result_path)
        .join("MiControl")
        .join(format!("elev_result_{request_id}.json"));

    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(150));
        if result_path.exists() {
            let content = std::fs::read_to_string(&result_path).unwrap();
            let v: Value = serde_json::from_str(&content).unwrap();
            println!("Result: ok={}", v["ok"].as_bool().unwrap_or(false));
            if v["ok"].as_bool() == Some(true) {
                println!(
                    "  Data: {}",
                    serde_json::to_string_pretty(&v["data"]).unwrap()
                );
            } else {
                println!("  Error: {}", v["error"].as_str().unwrap_or("?"));
            }
            std::fs::remove_file(&result_path).ok();
            std::fs::remove_file(&cmd_path).ok();
            return;
        }
    }
    println!("Timeout waiting for result!");
    std::fs::remove_file(&cmd_path).ok();
}
