# Reverse-engineering toolchain (this machine)

Installed 2026-11-27. Everything here is free; open-source licences noted where
they apply. Paths are absolute because these tools are not all on the system
PATH (scoop shims and the Hermes Node install are per-user).

## Inventory

| Tool | Version | Path / command | Licence |
|---|---|---|---|
| Ghidra | 12.1.4 PUBLIC | `C:\Users\mafsc\.tools\ghidra_12.1.4_PUBLIC` (`support\analyzeHeadless.bat`, `ghidraRun`) | Apache-2.0 |
| REA | 6.7.0 | `rea` (npm global), MCP server registered in Hermes as `rea` | MIT |
| JDK (required by Ghidra and jadx/apktool) | 21.0.11 | `C:\Program Files\Microsoft\jdk-21.0.11.10-hotspot` (`JAVA_HOME`) | GPLv2+CE |
| rizin | 0.8.x | `rizin`, `rz-bin` (scoop) | LGPL-3.0 |
| Cutter | 2.5.0 | `cutter` (scoop) | GPL-3.0 |
| radare2 | 6.2.6 | `radare2`, `r2` (scoop) | LGPL-3.0 |
| x64dbg | 2026.x | `x64dbg`, `x32dbg` (scoop) | GPL-3.0 |
| Sysinternals Suite | 20261105 | `procmon`, `procexp`, `strings`, `sigcheck`, `autoruns` (scoop) | Freeware (not OSS) |
| jadx | 1.5.7 | `jadx`, `jadx-gui` (scoop) | Apache-2.0 |
| apktool | 3.0.3 | `apktool` (scoop) | Apache-2.0 |
| Frida | 14.13.2 | `frida`, `frida-trace`, `frida-ps` (uv tool) | wxWindows/LGPL |
| ImHex | 1.38.1 | Start-menu shortcut (scoop app) | GPL-2.0 |
| binwalk | 2.1.0 (Python) | `binwalk` (uv tool) | MIT |
| capa | 10.0.0 | `capa` (uv tool) | Apache-2.0 |
| FLOSS | 3.2.0 | `floss` (uv tool) | Apache-2.0 |
| YARA | 4.6.0 | `yara`, `yarac` (scoop) | BSD-3-Clause |
| ACPICA (iasl, acpidump, acpixtract, acpiexec) | 20260408 | `C:\Users\mafsc\.tools\acpica`, shims `iasl`/`acpidump` | GPL-2.0 / Intel ACPI licence |
| Python RE libs | venv 3.12 | `C:\Users\mafsc\.tools\re-py\Scripts\python.exe` with capstone 5.0.9, pefile 2024.8.26, lief 1.0.0, yara-python 4.5.4 | mixed OSS |

Wireshark 4.6.6 (GPL-2.0) was already installed at `C:\Program Files\Wireshark`.

## Environment (User variables)

```
GHIDRA_INSTALL_DIR = C:\Users\mafsc\.tools\ghidra_12.1.4_PUBLIC
JAVA_HOME          = C:\Program Files\Microsoft\jdk-21.0.11.10-hotspot
```

Both are also set in the Hermes MCP entry for `rea` (`env:` block), because the
MCP child does not inherit variables added after the desktop app started.

Ghidra 12.1.4 refuses to run on JDK 17. There is a JDK 17 in
`%LOCALAPPDATA%\Programs\Microsoft\jdk-17.0.10.7-hotspot`; REA picks it up when
`JAVA_HOME` is unset, and the symptom is
`unsupported_version: Update JAVA_HOME ... 64-bit full JDK 21 or newer`.
Also keep `%APPDATA%\ghidra\ghidra_12.1.4_PUBLIC\java_home.save` pointing at the
JDK 21 root.

## ACPI tables (repeatable)

```bash
acpidump -b -o dsdt.dat            # or: acpidump -b  (writes every table)
acpixtract -a dsdt.dat             # split into dsdt.dat / ssdt*.dat
iasl -d dsdt.dat                   # -> dsdt.dsl
iasl -d ssdt1.dat                  # repeat per table
```

`iasl -d` also resolves external references when the sibling tables are in the
same directory. Keep the `.dsl` files next to the `.dat` files so `_SB.PCI0`
paths resolve instead of showing as external method calls.

The fan/clamshell analysis produced this way lives in
`docs/reverse-engineering/acpi-fan-clamshell.md` (worktree `re-acpi`).

## Native binaries (REA + Ghidra)

```bash
export GHIDRA_INSTALL_DIR="C:\Users\mafsc\.tools\ghidra_12.1.4_PUBLIC"
export JAVA_HOME="C:\Program Files\Microsoft\jdk-21.0.11.10-hotspot"
rea analyze "C:\Program Files\MiControl\ecram_service.exe" --provider ghidra --format json
```

Ghidra headless startup has a 330 s default deadline
(`REA_GHIDRA_STARTUP_TIMEOUT_MS`). Raise it for large targets or when the
machine is busy; a 500 KB PE that times out at 330 s is a load problem, not a
tool problem.

From an agent session, the same work goes through the `rea` MCP server
(`open_binary`, `analyze_function`, `binary_overview`, `export_evidence_bundle`).
Hermes must be restarted once after the MCP entry changes.

## Quick manual passes

```bash
rz-bin -I  target.exe        # headers, sections, imports
rz-bin -zz target.exe        # strings with addresses
capa -v target.exe           # capability detection (needs rules; ships with capa)
floss target.exe             # obfuscated string extraction
yara -r rules.yar target.exe
x64dbg target.exe            # dynamic analysis, symbols from Microsoft server
```

Android:

```bash
apktool d app.apk -o app-dec
jadx -d app-src app.apk
frida-ps -U                  # requires frida-server on the device
```

Firmware / containers:

```bash
binwalk -e image.bin
imhex                       # binary templates for structured formats
```

## Boundaries

- Sysinternals is freeware, not open source. It is here because Procmon and
  Sigcheck have no OSS equivalent for driver-level tracing on Windows.
- REA's jadx provider (`REA_JADX_MCP_JAR`) and IDA/Kuna providers are not
  configured. The jadx CLI works; the MCP bridge needs a caller-supplied
  `jadx-headless-mcp` JAR, which was not installed.
- Binwalk is v2 (Python). The Rust v3 is available via `cargo install binwalk`
  if extraction quality becomes a problem.
- Nothing here patches, signs, or bypasses driver signing. Analysing
  `IoTDriver.sys` is read-only.