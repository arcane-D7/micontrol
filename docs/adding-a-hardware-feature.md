# Adding a Hardware Feature

This guide walks through adding a new hardware module to the miPC HAL (Hardware Abstraction Layer).

## 7-Step Checklist

### 1. Create the HAL Module

Create `src-tauri/src/hw/your_feature.rs`:

```rust
//! Hardware access module for [your feature].

use crate::hw::errors::{HardwareError, HardwareResult};

/// Get the current state of [your feature].
pub fn get_your_feature_info() -> HardwareResult<YourFeatureInfo> {
    // Implementation
}

#[derive(serde::Serialize)]
pub struct YourFeatureInfo {
    pub value: u32,
}
```

### 2. Register the Module

Add `pub mod your_feature;` to `src-tauri/src/hw/mod.rs`.

### 3. Add Error Variants

Add error variants to `src-tauri/src/hw/errors.rs`:

```rust
pub enum HardwareError {
    // ... existing variants
    YourFeatureError(String),
}
```

### 4. Create Command Handlers

Add command functions in `src-tauri/src/commands/hardware.rs`:

```rust
#[tauri::command]
pub fn get_your_feature() -> Result<YourFeatureInfo, ErrorResponse> {
    hw::your_feature::get_your_feature_info()
        .map_err(|e| ErrorResponse::from(e))
}
```

### 5. Register Commands

Add the command to `generate_handler!` in `src-tauri/src/lib.rs`:

```rust
commands::hardware::get_your_feature,
```

### 6. Add Frontend Support

1. Add a hook in `src/hooks/useHardware.ts`
2. Add a UI component in `src/components/` or `src/pages/`
3. Add i18n keys to all locale files (`src/i18n/{en,pt,es,fr}.json`) — `pnpm run i18n:check` fails on a missing key
4. Add tests

### 7. Verify

```bash
pnpm run health-check
```

This is the same script CI runs: it compiles with `--features face`, denies clippy warnings, runs the Rust and frontend tests, type-checks, lints, checks i18n completeness and the kernel-handle leak gate.

## Testing

Add unit tests in the HAL module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_your_feature_info() {
        // Test the info structure
    }
}
```

Add integration tests in `src-tauri/tests/` if the feature involves the elevated bridge.

## Reverse Engineering & WORKING FORM

If your hardware feature involves direct EC RAM access, IOCTLs, or WMI method calls that have been reverse-engineered from the Xiaomi IoTDriver.sys or IoTService.exe:

1. **Document findings** — Add `// WORKING FORM — DO NOT MODIFY` comments above any code that has been verified against real hardware
2. **Reference the RE report** — Link to `docs/RE_ANALYSIS_REPORT.md` for technical context
3. **Do not modify verified patterns** — Buffer layouts, IOCTL codes, WMI method IDs, and API call sequences must not be changed without re-testing against the actual hardware
4. **Test against real hardware** — Use the `ecram_service.exe` CLI mode or the ECR Debug tab (dev only) to verify

See existing WORKING FORM comments in:

- `src-tauri/src/hw/battery.rs` — Battery health & AC adapter WMI queries
- `src-tauri/src/hw/ecram.rs` — ECRAM IOCTL buffer layout and pipe client
- `src-tauri/src/hw/fan.rs` — Performance mode WMI read/write
- `src-tauri/src/hw/wmi_ec.rs` — WMI EC read/write method calls
- `src-tauri/src/hw/wmi_cache.rs` — WMI connection caching pattern
