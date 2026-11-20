//! Central error catalog for MiControl.
//!
//! Contract: `docs/architecture/UNIVERSAL-CONTRACTS.md` section 2. Every error
//! born in `src-tauri/src` is classified here before it can be surfaced:
//! [`catalog::AppError`] wraps a [`catalog::ErrorCode`], and each code has a
//! stable log id, an i18n key, a severity, a retry policy and a suggested
//! user action. Domain errors keep their own types (e.g. `HardwareError`)
//! and map into the catalog through `impl HardwareError { fn code() }`.
//!
//! Gated by `scripts/check-error-catalog.mjs`: enum/catalog parity, unique
//! `domain.snake_case` ids, `errors.<id>` keys present in all four locales,
//! and a ratchet that blocks new string-typed error returns.

pub mod catalog;

pub use catalog::{AppError, ErrorCode, ErrorSpec, RetryPolicy, Severity, UserAction, CATALOG};

#[cfg(test)]
mod tests;
