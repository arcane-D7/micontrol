# Sprint 17 — P1 Security & DevOps Hardening

**Sprint ID:** S17
**Priority:** P1 — HIGH (Fix before/shortly after release)
**Estimated tickets:** 15
**Estimated effort:** 2–3 days
**Base branch:** `master` (after S16 merge)
**Source:** `docs/stability-report-2026-06-24-post-sprints-13-15.md` — P1 Recommendations #9–#18

---

## Sprint Goal

Address all HIGH severity security and DevOps findings. Harden the elevated bridge, WiFi password encryption, CI/CD pipeline, and repository health files. These issues don't block release but should be fixed before or shortly after launch.

---

## Tickets

### S17-01: Fix script path allowlist bypass (ends_with)

**Severity:** HIGH
**Finding:** S2 — `validate_script_path` accepts `C:\Users\attacker\bin\cmd.exe`
**Files:** `src-tauri/src/hw/hotkeys.rs`
**Tasks:**

- [x] Replace `ends_with()` check with canonical path resolution
- [x] Use `std::fs::canonicalize()` to resolve the full path
- [x] Compare against a list of canonical allowed paths (e.g., `C:\Windows\System32\cmd.exe`)
- [x] Add test for path traversal attack (`..\..\..\cmd.exe`)
- [x] Add test for legitimate `cmd.exe` path
      **Acceptance:** Only canonical Windows system paths are allowed. Path traversal attacks are blocked.

---

### S17-02: Replace XOR with AES-256-GCM for WiFi password encryption

**Severity:** HIGH
**Finding:** S1, V5 — XOR is malleable, no authentication tag
**Files:** `src-tauri/src/hw/iotservice.rs`, `src-tauri/Cargo.toml`
**Tasks:**

- [x] Add `aes-gcm` crate to `Cargo.toml`
- [x] Derive a dedicated WiFi encryption key from the HMAC master key using HKDF-SHA256
- [x] Replace XOR encrypt/decrypt functions with AES-256-GCM
- [x] Handle migration: if old XOR-encrypted data exists, decrypt with XOR, re-encrypt with AES-GCM
- [x] Add nonce generation (random 12 bytes per encryption)
- [x] Add tests for encrypt/decrypt round-trip
- [x] Add test for tamper detection (bit-flip should fail)
      **Acceptance:** WiFi passwords are encrypted with AES-256-GCM. Tampering is detected.

---

### S17-03: Make Authenticode signing failures blocking in CI

**Severity:** HIGH
**Finding:** D2 — Failed signing produces unsigned release silently
**Files:** `.github/workflows/release.yml`
**Tasks:**

- [x] Remove `|| echo "::warning::..."` fallback on signtool commands
- [x] Add explicit error check: `if [ $? -ne 0 ]; then echo "::error::Signing failed"; exit 1; fi`
- [x] Add a post-signing verification step: `signtool verify /pa /v "$MSI_PATH"`
- [x] Only sign if `WINDOWS_CERTIFICATE_PASSWORD` secret is present (skip gracefully if not set)
- [x] Add job output indicating whether signing was performed
      **Acceptance:** Release workflow fails if signing fails. Unsigned releases cannot be published.

---

### S17-04: Create .env.example file

**Severity:** HIGH
**Finding:** D3 — No documentation of required env vars
**Files:** `.env.example` (new)
**Tasks:**

- [x] Document all environment variables used by the app:
  - `VITE_SENTRY_DSN` — Sentry DSN for crash reporting (optional)
  - `OPENAI_API_KEY` — Not needed (stored in keyring)
  - `WINDOWS_CERTIFICATE_PASSWORD` — For release signing (CI only)
- [x] Add comments explaining each variable
- [x] Add `.env.example` to git
- [x] Ensure `.env` is in `.gitignore` (verify it already is)
      **Acceptance:** `.env.example` exists and documents all environment variables.

---

### S17-05: Fix npm audit enforcement in CI

**Severity:** MEDIUM
**Finding:** D5 — `continue-on-error: true` hides vulnerabilities
**Files:** `.github/workflows/ci.yml`
**Tasks:**

- [x] Remove `continue-on-error: true` from `npm audit` step
- [x] Add `--audit-level=high` flag to only fail on HIGH+ vulnerabilities
- [x] Add `--production` flag to skip devDependencies
- [x] Add a comment explaining the audit level threshold
      **Acceptance:** CI fails on HIGH+ npm vulnerabilities. LOW/MEDIUM are reported but don't block.

---

### S17-06: Fix coverage job silent failures

**Severity:** MEDIUM
**Finding:** D6 — Coverage failures are silent
**Files:** `.github/workflows/ci.yml`
**Tasks:**

- [x] Remove `continue-on-error: true` from coverage jobs
- [x] Add coverage threshold check (e.g., fail if coverage < 40%)
- [x] Upload coverage report as artifact
- [x] Add coverage badge to README (after S17-13 URL fix)
      **Acceptance:** Coverage failures are visible. Coverage reports are uploaded as artifacts.

---

### S17-07: Add SECURITY.md

**Severity:** LOW (promoted for responsible disclosure)
**Finding:** D13
**Files:** `SECURITY.md` (new)
**Tasks:**

- [x] Create `SECURITY.md` with:
  - Supported versions table
  - Vulnerability reporting process (email or GitHub Security Advisories)
  - Response timeline (e.g., 48h acknowledgment, 90h disclosure)
  - PGP key or contact method
  - Scope (what is/isn't a vulnerability)
    **Acceptance:** `SECURITY.md` exists with responsible disclosure policy.

---

### S17-08: Add CODE_OF_CONDUCT.md

**Severity:** LOW
**Finding:** D14 — Referenced in README but missing
**Files:** `CODE_OF_CONDUCT.md` (new)
**Tasks:**

- [x] Use Contributor Covenant v2.1 template
- [x] Customize contact email/address
- [x] Verify README link works
      **Acceptance:** `CODE_OF_CONDUCT.md` exists. README link resolves.

---

### S17-09: Add PR template

**Severity:** MEDIUM
**Finding:** D8
**Files:** `.github/PULL_REQUEST_TEMPLATE.md` (new)
**Tasks:**

- [x] Create PR template with:
  - Summary of changes
  - Related issue (closes #N)
  - Type of change (bug fix, feature, breaking change, docs)
  - Checklist (tests added, docs updated, changelog updated)
  - Breaking changes section
  - Screenshots (if UI change)
    **Acceptance:** PR template appears when creating a new PR.

---

### S17-10: Add issue templates

**Severity:** MEDIUM
**Finding:** D9
**Files:** `.github/ISSUE_TEMPLATE/bug_report.yml`, `.github/ISSUE_TEMPLATE/feature_request.yml`, `.github/ISSUE_TEMPLATE/config.yml`
**Tasks:**

- [x] Create bug report template (YAML form) with: description, steps to reproduce, expected vs actual, environment (OS, version), logs
- [x] Create feature request template with: problem statement, proposed solution, alternatives, additional context
- [x] Create `config.yml` to disable blank issues and add contact links
      **Acceptance:** Issue templates appear when creating a new issue. Blank issues are disabled.

---

### S17-11: Add Sentry before_send callback for PII stripping

**Severity:** HIGH
**Finding:** R5 — No PII stripping before sending to Sentry
**Files:** `src-tauri/src/lib.rs`
**Tasks:**

- [x] Add `before_send` callback to Sentry init
- [x] Strip PII fields from events:
  - Computer name / hostname
  - User IP address (Sentry can do this server-side, but also strip client-side)
  - File paths containing usernames (`C:\Users\{name}\...`)
  - Any environment variables in stack traces
- [x] Replace `C:\Users\{username}\` with `C:\Users\<redacted>\` in breadcrumbs and contexts
- [x] Add test for PII stripping logic
      **Acceptance:** Sentry events do not contain PII. Usernames and paths are redacted.

---

### S17-12: Add URL validation using url crate

**Severity:** MEDIUM
**Finding:** S3 — Basic prefix check for URL validation
**Files:** `src-tauri/src/hw/hotkeys.rs`, `src-tauri/Cargo.toml`
**Tasks:**

- [x] Add `url` crate to `Cargo.toml`
- [x] Replace prefix-based URL validation with `url::Url::parse()`
- [x] Only allow `http://` and `https://` schemes
- [x] Block `file://`, `javascript:`, `data:`, and other dangerous schemes
- [x] Add tests for valid/invalid URLs
      **Acceptance:** URL validation uses proper parsing. Dangerous schemes are blocked.

---

### S17-13: Fix README badges and URLs

**Severity:** HIGH (promoted — already partially in S16-13, but this covers remaining URLs)
**Finding:** D4
**Files:** `README.md`
**Tasks:**

- [x] Fix CI badge: `github.com/user/miPC` → `github.com/Freitas-MA/miPC`
- [x] Fix branch in badge: `branch=main` → `branch=master`
- [x] Fix version badge URL
- [x] Fix clone URL
- [x] Fix releases link
- [x] Fix issues link
- [x] Remove MSI references in README (app uses NSIS)
- [x] Verify all links resolve
      **Acceptance:** All README URLs point to correct repository. Badges display correctly.

---

### S17-14: Remove dead MSI references in release.yml

**Severity:** LOW
**Finding:** D15
**Files:** `.github/workflows/release.yml`
**Tasks:**

- [x] Remove MSI signing block (only NSIS is used)
- [x] Remove MSI path search
- [x] Keep only NSIS/EXE signing
- [x] Update `latest.json` manifest to only reference NSIS installer
      **Acceptance:** Release workflow only handles NSIS installer. No MSI dead code.

---

### S17-15: Health check verification and commit

**Severity:** N/A (process)
**Tasks:**

- [x] Run all 9 health checks
- [x] Fix any failures
- [x] Commit with message: `feat(sprint-17): security and devops hardening (P1)`
- [x] Verify no regressions in test count
      **Acceptance:** 9/9 health checks pass. No regressions.

---

## Sprint Exit Criteria

- [x] Script path allowlist uses canonical paths
- [x] WiFi passwords use AES-256-GCM
- [x] Authenticode signing is blocking in CI
- [x] `.env.example` exists
- [x] `npm audit` enforced in CI
- [x] `SECURITY.md` exists
- [x] `CODE_OF_CONDUCT.md` exists
- [x] PR template exists
- [x] Issue templates exist
- [x] Sentry strips PII
- [x] URL validation uses `url` crate
- [x] README URLs are correct
- [x] No MSI dead code in release.yml
- [x] 9/9 health checks pass
