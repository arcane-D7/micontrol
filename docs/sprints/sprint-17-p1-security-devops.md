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

- [ ] Replace `ends_with()` check with canonical path resolution
- [ ] Use `std::fs::canonicalize()` to resolve the full path
- [ ] Compare against a list of canonical allowed paths (e.g., `C:\Windows\System32\cmd.exe`)
- [ ] Add test for path traversal attack (`..\..\..\cmd.exe`)
- [ ] Add test for legitimate `cmd.exe` path
      **Acceptance:** Only canonical Windows system paths are allowed. Path traversal attacks are blocked.

---

### S17-02: Replace XOR with AES-256-GCM for WiFi password encryption

**Severity:** HIGH
**Finding:** S1, V5 — XOR is malleable, no authentication tag
**Files:** `src-tauri/src/hw/iotservice.rs`, `src-tauri/Cargo.toml`
**Tasks:**

- [ ] Add `aes-gcm` crate to `Cargo.toml`
- [ ] Derive a dedicated WiFi encryption key from the HMAC master key using HKDF-SHA256
- [ ] Replace XOR encrypt/decrypt functions with AES-256-GCM
- [ ] Handle migration: if old XOR-encrypted data exists, decrypt with XOR, re-encrypt with AES-GCM
- [ ] Add nonce generation (random 12 bytes per encryption)
- [ ] Add tests for encrypt/decrypt round-trip
- [ ] Add test for tamper detection (bit-flip should fail)
      **Acceptance:** WiFi passwords are encrypted with AES-256-GCM. Tampering is detected.

---

### S17-03: Make Authenticode signing failures blocking in CI

**Severity:** HIGH
**Finding:** D2 — Failed signing produces unsigned release silently
**Files:** `.github/workflows/release.yml`
**Tasks:**

- [ ] Remove `|| echo "::warning::..."` fallback on signtool commands
- [ ] Add explicit error check: `if [ $? -ne 0 ]; then echo "::error::Signing failed"; exit 1; fi`
- [ ] Add a post-signing verification step: `signtool verify /pa /v "$MSI_PATH"`
- [ ] Only sign if `WINDOWS_CERTIFICATE_PASSWORD` secret is present (skip gracefully if not set)
- [ ] Add job output indicating whether signing was performed
      **Acceptance:** Release workflow fails if signing fails. Unsigned releases cannot be published.

---

### S17-04: Create .env.example file

**Severity:** HIGH
**Finding:** D3 — No documentation of required env vars
**Files:** `.env.example` (new)
**Tasks:**

- [ ] Document all environment variables used by the app:
  - `VITE_SENTRY_DSN` — Sentry DSN for crash reporting (optional)
  - `OPENAI_API_KEY` — Not needed (stored in keyring)
  - `WINDOWS_CERTIFICATE_PASSWORD` — For release signing (CI only)
- [ ] Add comments explaining each variable
- [ ] Add `.env.example` to git
- [ ] Ensure `.env` is in `.gitignore` (verify it already is)
      **Acceptance:** `.env.example` exists and documents all environment variables.

---

### S17-05: Fix npm audit enforcement in CI

**Severity:** MEDIUM
**Finding:** D5 — `continue-on-error: true` hides vulnerabilities
**Files:** `.github/workflows/ci.yml`
**Tasks:**

- [ ] Remove `continue-on-error: true` from `npm audit` step
- [ ] Add `--audit-level=high` flag to only fail on HIGH+ vulnerabilities
- [ ] Add `--production` flag to skip devDependencies
- [ ] Add a comment explaining the audit level threshold
      **Acceptance:** CI fails on HIGH+ npm vulnerabilities. LOW/MEDIUM are reported but don't block.

---

### S17-06: Fix coverage job silent failures

**Severity:** MEDIUM
**Finding:** D6 — Coverage failures are silent
**Files:** `.github/workflows/ci.yml`
**Tasks:**

- [ ] Remove `continue-on-error: true` from coverage jobs
- [ ] Add coverage threshold check (e.g., fail if coverage < 40%)
- [ ] Upload coverage report as artifact
- [ ] Add coverage badge to README (after S17-13 URL fix)
      **Acceptance:** Coverage failures are visible. Coverage reports are uploaded as artifacts.

---

### S17-07: Add SECURITY.md

**Severity:** LOW (promoted for responsible disclosure)
**Finding:** D13
**Files:** `SECURITY.md` (new)
**Tasks:**

- [ ] Create `SECURITY.md` with:
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

- [ ] Use Contributor Covenant v2.1 template
- [ ] Customize contact email/address
- [ ] Verify README link works
      **Acceptance:** `CODE_OF_CONDUCT.md` exists. README link resolves.

---

### S17-09: Add PR template

**Severity:** MEDIUM
**Finding:** D8
**Files:** `.github/PULL_REQUEST_TEMPLATE.md` (new)
**Tasks:**

- [ ] Create PR template with:
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

- [ ] Create bug report template (YAML form) with: description, steps to reproduce, expected vs actual, environment (OS, version), logs
- [ ] Create feature request template with: problem statement, proposed solution, alternatives, additional context
- [ ] Create `config.yml` to disable blank issues and add contact links
      **Acceptance:** Issue templates appear when creating a new issue. Blank issues are disabled.

---

### S17-11: Add Sentry before_send callback for PII stripping

**Severity:** HIGH
**Finding:** R5 — No PII stripping before sending to Sentry
**Files:** `src-tauri/src/lib.rs`
**Tasks:**

- [ ] Add `before_send` callback to Sentry init
- [ ] Strip PII fields from events:
  - Computer name / hostname
  - User IP address (Sentry can do this server-side, but also strip client-side)
  - File paths containing usernames (`C:\Users\{name}\...`)
  - Any environment variables in stack traces
- [ ] Replace `C:\Users\{username}\` with `C:\Users\<redacted>\` in breadcrumbs and contexts
- [ ] Add test for PII stripping logic
      **Acceptance:** Sentry events do not contain PII. Usernames and paths are redacted.

---

### S17-12: Add URL validation using url crate

**Severity:** MEDIUM
**Finding:** S3 — Basic prefix check for URL validation
**Files:** `src-tauri/src/hw/hotkeys.rs`, `src-tauri/Cargo.toml`
**Tasks:**

- [ ] Add `url` crate to `Cargo.toml`
- [ ] Replace prefix-based URL validation with `url::Url::parse()`
- [ ] Only allow `http://` and `https://` schemes
- [ ] Block `file://`, `javascript:`, `data:`, and other dangerous schemes
- [ ] Add tests for valid/invalid URLs
      **Acceptance:** URL validation uses proper parsing. Dangerous schemes are blocked.

---

### S17-13: Fix README badges and URLs

**Severity:** HIGH (promoted — already partially in S16-13, but this covers remaining URLs)
**Finding:** D4
**Files:** `README.md`
**Tasks:**

- [ ] Fix CI badge: `github.com/user/miPC` → `github.com/Freitas-MA/miPC`
- [ ] Fix branch in badge: `branch=main` → `branch=master`
- [ ] Fix version badge URL
- [ ] Fix clone URL
- [ ] Fix releases link
- [ ] Fix issues link
- [ ] Remove MSI references in README (app uses NSIS)
- [ ] Verify all links resolve
      **Acceptance:** All README URLs point to correct repository. Badges display correctly.

---

### S17-14: Remove dead MSI references in release.yml

**Severity:** LOW
**Finding:** D15
**Files:** `.github/workflows/release.yml`
**Tasks:**

- [ ] Remove MSI signing block (only NSIS is used)
- [ ] Remove MSI path search
- [ ] Keep only NSIS/EXE signing
- [ ] Update `latest.json` manifest to only reference NSIS installer
      **Acceptance:** Release workflow only handles NSIS installer. No MSI dead code.

---

### S17-15: Health check verification and commit

**Severity:** N/A (process)
**Tasks:**

- [ ] Run all 9 health checks
- [ ] Fix any failures
- [ ] Commit with message: `feat(sprint-17): security and devops hardening (P1)`
- [ ] Verify no regressions in test count
      **Acceptance:** 9/9 health checks pass. No regressions.

---

## Sprint Exit Criteria

- [ ] Script path allowlist uses canonical paths
- [ ] WiFi passwords use AES-256-GCM
- [ ] Authenticode signing is blocking in CI
- [ ] `.env.example` exists
- [ ] `npm audit` enforced in CI
- [ ] `SECURITY.md` exists
- [ ] `CODE_OF_CONDUCT.md` exists
- [ ] PR template exists
- [ ] Issue templates exist
- [ ] Sentry strips PII
- [ ] URL validation uses `url` crate
- [ ] README URLs are correct
- [ ] No MSI dead code in release.yml
- [ ] 9/9 health checks pass
