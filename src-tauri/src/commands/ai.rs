//! Backend AI analysis command.
//!
//! Keeps the AI provider API key in the backend keyring; never exposes
//! it to the frontend.

use keyring::Entry;
use std::time::Duration;

const KEYRING_SERVICE: &str = "com.mipc.micontrol";
const KEYRING_USER: &str = "openai_api_key";
const TELEMETRY_CONSENT_KEY: &str = "telemetry_consent";

/// Analyze system data using the AI provider.
/// The API key is read from the keyring in the backend — never exposed to the frontend.
#[tauri::command]
pub async fn analyze_system(
    system_context: String,
    base_url: String,
    model: String,
) -> Result<String, String> {
    // Check telemetry consent before proceeding
    let consent = get_telemetry_consent().map_err(|e| e.to_string())?;
    if consent != "granted" {
        return Err("consent_denied".to_string());
    }

    // Read API key from keyring
    let entry =
        Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| format!("Keyring error: {e}"))?;
    let api_key = entry
        .get_password()
        .map_err(|e| format!("Failed to read API key: {e}"))?;

    // Build the request
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You are a hardware analysis assistant. Use inclusive, accessible language. Avoid jargon, slang, or culturally specific idioms that may exclude users. Note: AI-generated content may be inaccurate. Verify critical information before acting on it."},
            {"role": "user", "content": system_context}
        ],
        "max_tokens": 1000,
    });

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("API error ({}): {}", status, text));
    }

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    // Extract the assistant's message
    let content = result["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("No content in response")?
        .to_string();

    // Track usage — approximate estimation based on I/O
    let input_tokens = system_context.len() as u64 / 4; // rough char→token estimate
    let output_tokens = content.len() as u64 / 4;
    crate::util::ai_usage::record_usage(input_tokens, output_tokens);

    Ok(content)
}

/// Quick connectivity + auth test — sends a minimal prompt.
#[tauri::command]
pub async fn test_connection(base_url: String, model: String) -> Result<String, String> {
    let entry =
        Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| format!("Keyring error: {e}"))?;
    let api_key = entry
        .get_password()
        .map_err(|e| format!("Failed to read API key: {e}"))?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Reply with the single word OK."}],
        "max_tokens": 5,
    });

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("API error ({}): {}", status, text));
    }

    Ok("ok".to_string())
}

/// Get AI usage statistics.
#[tauri::command]
pub fn get_ai_usage() -> crate::util::ai_usage::AiUsageStats {
    crate::util::ai_usage::get_usage()
}

/// Reset AI usage statistics.
#[tauri::command]
pub fn reset_ai_usage() {
    crate::util::ai_usage::reset_usage();
}

/// Check telemetry consent from the credential store.
fn get_telemetry_consent() -> Result<String, Box<dyn std::error::Error>> {
    let entry = Entry::new(KEYRING_SERVICE, TELEMETRY_CONSENT_KEY)?;
    match entry.get_password() {
        Ok(val) => {
            let parsed: serde_json::Value = serde_json::from_str(&val)?;
            Ok(parsed["value"].as_str().unwrap_or("denied").to_string())
        }
        Err(_) => Ok("denied".to_string()),
    }
}
