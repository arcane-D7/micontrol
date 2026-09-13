# PRODUCT.md — MiControl

## Users

- **Owners of Xiaomi gaming laptops** (e.g. Mi Gaming 2019 / Redmi G) running Windows 11.
- Technical-comfortable enthusiasts, but not developers: they install, click, and expect things to work.
- Use cases: fan/performance control, battery care, thermal monitoring, RGB keyboard, screen settings (HDR, brightness, adaptive), touchpad gestures, hotkeys, debloating, phone integration (Cross Device), AI assistant features, face unlock.
- Many are on **Portuguese (pt-BR), Spanish, French, or English** locales — full i18n required.
- Often used **plugged in at a desk at night** (gaming, dark rooms) and **on battery on the go**.

## Product Purpose

MiControl is the missing control center Xiaomi never shipped for these laptops: a single desktop app (Tauri 2 + React) that talks to the EC via IoTDriver.sys to expose fan curves, power modes, battery charge limits, thermal readings, and device settings that Windows alone can't reach. It replaces the buggy official "Mi Gaming Utility" and integrates modern features (AI assistant, face unlock, cross-device).

## Brand & Tone

- **Not** corporate/enterprise. **Not** gamer-RGB-splash either.
- Modern, dark-first hardware utility: confident, precise, calm. Think "well-engineered tool" — Machinary/Linear-adjacent density, subtle depth, hardware-informed accents.
- Data (temps, watts, RPM) is the hero: numbers must be legible, tabular, never decorative.
- Feedback language: short, human, actionable ("Fan curve updated", not "Operation completed successfully").

## Anti-references

- The old **Mi Gaming Utility**: buggy, ugly, unreliable.
- Generic gamer RGB splash dashboards with useless charts.
- Windows Settings-style blandness with no hierarchy.

## Strategic Principles

1. **Hardware truth first** — every value shown must come from real sensors; never fabricate fallbacks (regression S35 taught us this).
2. **Autonomous reliability** — elevated ops must never prompt UAC in normal flow; the bridge service handles it.
3. **Respect the machine** — safe defaults, reversible actions, explicit risk tiers for debloat.
4. **Dark-first, i18n-complete** — dark theme is primary; all strings localized (en/es/fr/pt).
5. **Calm density** — dense dashboards OK, but with clear hierarchy, breathing room, and zero visual noise.

## Register

`product`
