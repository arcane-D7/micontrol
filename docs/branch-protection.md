# Branch Protection Rules

This document describes the required branch protection configuration for the `main` branch to ensure code quality and prevent unauthorized changes.

## Required Status Checks

The following CI checks MUST pass before a pull request can be merged to `main`:

| Check           | Job           | Description                                                          |
| --------------- | ------------- | -------------------------------------------------------------------- |
| `rust`          | CI / rust     | `cargo fmt --check`, `cargo check`, `cargo clippy`, `cargo test`     |
| `frontend`      | CI / frontend | `tsc --noEmit`, `eslint`, `prettier --check`, `vite build`           |
| `version:check` | CI / version  | Ensures package.json, Cargo.toml, and tauri.conf.json versions agree |

### Future Checks (when implemented)

| Check         | Description                                       |
| ------------- | ------------------------------------------------- |
| `cargo audit` | Scans Rust dependencies for known CVEs            |
| `npm audit`   | Scans npm dependencies for known vulnerabilities  |
| `tauri-smoke` | Builds a Tauri release bundle to verify packaging |

## Required Reviews

- **Minimum reviewers**: 1 (for small teams)
- **Dismiss stale approvals**: Yes — when new commits are pushed, existing approvals are dismissed
- **Require review from code owners**: Yes (when CODEOWNERS file is added)

## Branch Protection Rules

### `main` branch

1. **Require pull request before merging**
   - Require at least 1 approval
   - Dismiss stale pull request approvals when new commits are pushed

2. **Require status checks to pass**
   - Require branches to be up to date before merging
   - Required checks: `rust`, `frontend`, `version:check`

3. **Require conversation resolution**
   - All conversations on the PR must be resolved before merging

4. **Do not allow bypassing the above settings**
   - Administrators must follow the same rules

5. **Restrict who can push to matching branches**
   - No direct pushes to `main` — all changes via pull request

## Setup Instructions

### Via GitHub Web UI

1. Navigate to repository **Settings** → **Branches**
2. Click **Add rule** under "Branch protection rules"
3. Branch name pattern: `main`
4. Enable:
   - ☑ Require a pull request before merging (set required reviewers to 1)
   - ☑ Require status checks to pass before merging
     - Select: `rust`, `frontend`, `version:check`
   - ☑ Require conversation resolution before merging
   - ☑ Do not allow bypassing the above settings

### Via GitHub CLI

```bash
gh api repos/{owner}/{repo}/branches/main/protection \
  --method PUT \
  --field required_pull_request_reviews='{"required_approving_review_count":1,"dismiss_stale_reviews":true}' \
  --field required_status_checks='{"strict":true,"contexts":["rust","frontend","version:check"]}' \
  --field enforce_admins=true \
  --field restrictions=null
```

## CODEOWNERS

Create a `CODEOWNERS` file in the repository root:

```
# Default owner
* @mafsc

# Rust backend
/src-tauri/ @mafsc

# Frontend
/src/ @mafsc

# CI/CD
/.github/ @mafsc

# Documentation
/docs/ @mafsc
```
