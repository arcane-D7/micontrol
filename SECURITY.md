# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.0.x   | :white_check_mark: |
| < 1.0   | :x:                |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security vulnerability in miPC, please report it responsibly.

### How to Report

1. **Do NOT open a public GitHub issue.**
2. Email: security@freitas-ma.dev (or use [GitHub Security Advisories](https://github.com/Freitas-MA/miPC/security/advisories/new))
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

### Response Timeline

| Action             | Timeline              |
| ------------------ | --------------------- |
| Acknowledgment     | Within 48 hours       |
| Initial assessment | Within 7 days         |
| Fix or mitigation  | Within 90 days        |
| Public disclosure  | After fix is released |

### Scope

**In scope:**

- The miPC desktop application (Rust backend + React frontend)
- The elevated bridge subprocess
- IPC between frontend and backend
- Credential storage (keyring)
- WiFi password storage
- HMAC authentication

**Out of scope:**

- Vulnerabilities in third-party dependencies (report to upstream)
- Social engineering attacks
- Physical access to an unlocked machine
- DoS attacks requiring local access

### Recognition

We appreciate responsible disclosure and will credit researchers in our security advisories (unless they prefer to remain anonymous).
