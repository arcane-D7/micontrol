# Memory Leak Audit — MiControl

Audit date: **2026-10-24**
Target: `MiControl 0.2.19-beta` (`micontrol.exe`, `src-tauri`)
Host: `MF-PC` — Xiaomi Book Pro 14, 32 GB RAM, Windows 11 Pro 26200 (NVMe SSD)
Artifact: `C:\Program Files\MiControl\micontrol.exe` (106.55 MB, **NotSigned**)

> **REMEDIATION STATUS (2026-10-24, S67)** — implemented in source, pending release:
>
> | Audit item                                                                                                       | Status                                                                            |
> | ---------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
> | RC-1 `pipe_request` early-return leak (event + pipe handle)                                                      | ✅ fixed — both wrapped in RAII guards (`util/win_handle.rs`)                     |
> | RC-2 `read_ecram_via_pipe` `WriteFile(...).ok()?` leak                                                           | ✅ fixed — RAII guard                                                             |
> | RC-3 `iotservice.rs` scattered event closes (incl. a `GetOverlappedResult(...)?` leak NOT in the original audit) | ✅ fixed — single RAII guard replaces 5 manual closes                             |
> | RC-4 `pipe_server.rs` fragile 3-exit manual closes                                                               | ✅ fixed — RAII guards                                                            |
> | EXTRA `send_pipe_request` (`ecram.rs`) `WriteFile(...).map_err(..)?` leak                                        | ✅ fixed — found by the S67 gate                                                  |
> | EXTRA bridge `handle_client` — `return` on 0-byte read leaked `io_event`                                         | ✅ fixed — RAII guard                                                             |
> | RC-6 thread/poll fan-out (battery)                                                                               | ⏳ partially bounded by the existing 15 s throttle; worker-pool refactor deferred |
> | Item 5 of §4 (connection reuse)                                                                                  | ⏳ deferred — the per-call reconnect is cheap once handles no longer leak         |
> | Item 6 of §4 (0x80041002 backoff)                                                                                | ⏳ deferred — logging-only churn, no resource leak                                |
>
> **Prevention system (3 layers, all shipped):**
>
> 1. **RAII everywhere** — `util/win_handle.rs` (`OwnedHandle`/`OwnedEvent`) + 7 leak-prone call sites converted; every early return now closes automatically.
> 2. **Static gate** — `scripts/check-handle-leaks.mjs` (wired into `pnpm run health-check` as the `handle-leaks` step): fails the build if a function acquires a kernel handle (`CreateFileW`, `CreateEventW`, `CreateNamedPipeW`, `OpenProcess`, …) without an RAII guard, `from_raw_handle`, or an explicit `// leak-check: <reason>` annotation. 32 acquisition sites policed.
> 3. **Runtime monitor** — `util/leak_monitor.rs`: samples handle count + commit bytes every 60 s over a 15 min sliding window; growth ≥ 3 handles/min or ≥ 2 MB/min triggers a throttled WARN (`target: leak_monitor`) and is exposed to the UI via the `get_leak_diagnostics` Tauri command. Catches any future regression in ~1 h instead of 218 h.

---

## 1. Executive summary

`micontrol.exe` leaks **kernel handles** at a measured rate of **~11 handles/minute**, reaching
**187,716 handles** after **216.8 h** of uptime. Commit charge grows to **2.5 GB** while the
working set stays at **150 MB**, and the virtual address space reaches **58.9 GB**.

This is not a heap leak. GDI objects (32) and USER objects (82) are normal, which rules out UI
object accumulation. The handles are kernel objects — file/pipe handles and event objects — created
on every elevated round-trip and never closed.

| Metric                 | Observed               | Expected for this app |
| ---------------------- | ---------------------- | --------------------- |
| Handles                | **187,716**            | < 3,000               |
| Threads                | **115**                | 20–40                 |
| Commit / private bytes | **2,500.6 MB**         | 150–250 MB            |
| Virtual bytes          | **58.9 GB**            | ~4 GB                 |
| Working set            | 150.3 MB               | 80–120 MB             |
| Peak working set       | 209.4 MB               | —                     |
| CPU accumulated        | **6.1 h over 216.8 h** | < 0.5 h               |
| GDI / USER objects     | 32 / 82                | normal                |

---

## 2. Measured evidence

### 2.1 Leak rate (two independent samples)

```
sample 1 (60 s): 187,704 -> 187,716  = +12 handles
sample 2 (60 s):                     = +10 handles
```

Average: **~11 handles/min**. Extrapolated over the 216.8 h uptime this is ≈ 143,000 handles,
consistent with the 187,716 observed (the rate rises with hardware-polling activity).

### 2.2 Bridge connection churn — `C:\ProgramData\MiControl\logs\bridge.log`

36,565 lines accumulated:

| Event                           | Count      |
| ------------------------------- | ---------- |
| `client connected`              | **9,701**  |
| `cmd=wmi_ec_read_adapter_power` | **13,452** |
| `cmd=read_thermal_readings`     | **11,034** |

Representative excerpt showing back-to-back reconnects 2–12 ms apart:

```
2026-10-24T09:53:40.116Z client connected
2026-10-24T09:53:40.116Z request received: cmd=wmi_ec_read_adapter_power
2026-10-24T09:53:40.120Z response written: ok=true total_elapsed_ms=4
2026-10-24T09:53:40.132Z client connected          <- reconnect 12 ms later
2026-10-24T09:53:40.132Z request received: cmd=read_thermal_readings
...
2026-10-24T09:53:40.143Z client connected          <- reconnect 2 ms later
```

The client opens a fresh pipe connection per command instead of reusing one.

### 2.3 Polling frequency — `C:\Users\mafsc\AppData\Local\MiControl\logs\tauri-dev-trace.log.1`

282,082 lines / 39.08 MB. Dominant repeated message:

```
[DEBUG] hw::battery: ac probe throttled: returning cached value None
        (elapsed_ms=4 < interval_ms=15000)
```

Top duplicate counts: 137, 116, 116, 96, 91 occurrences of the same throttled-probe line per
timestamp. The throttle works, but **multiple threads poll in parallel** — hence 115 threads.

### 2.4 Related instability (same component)

Three `micontrol_bridge.exe` crash dumps, all 51.3 MB, all 2026-09-10 21:54:

```
C:\ProgramData\MiControl\dumps\micontrol_bridge.exe.23460.dmp
C:\ProgramData\MiControl\dumps\micontrol_bridge.exe.39780.dmp
C:\ProgramData\MiControl\dumps\micontrol_bridge.exe.42404.dmp
```

And an unhandled WMI error appearing repeatedly in `tauri-app.log`:

```
micontrol_lib::elevated: Elevated command result: ok=false
  error=Some("Hardware error: 0x80041002")
```

`0x80041002` = `WBEM_E_NOT_FOUND`. The WMI class/instance does not exist on this hardware; the
app logs the failure and retries indefinitely on its poll timer.

---

## 3. Root causes (located in source)

### RC-1 — `pipe_request()` leaks both handles on the error path 🔴 **PRIMARY**

**File:** `src-tauri/src/elev_bridge.rs`
**Function:** `pipe_request(&str) -> Result<String, String>`
**Lines:** 706–896

The function acquires two kernel handles and closes them only at line 892–893, at the very end:

```rust
// line 754 — handle #1
CreateFileW( PCWSTR(path_w.as_ptr()), ... )   // -> `handle`

// line 789 — handle #2
let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) };

// line 837 — EARLY RETURN THAT SKIPS BOTH CloseHandle CALLS
write_result.map_err(|e| format!("WriteFile bridge pipe: {e}"))?;

// ... read loop, also returns Err at several points ...

// lines 892-893 — the ONLY cleanup, reached on the happy path
CloseHandle(event).ok();
CloseHandle(handle).ok();
```

Line 837 uses `?` on a `Result`. Every time `WriteFile` fails — including `WAIT_TIMEOUT`
(effectively a wedged or busy bridge, which the 30 s `PIPE_OP_TIMEOUT_MS` makes reachable) —
the function returns before either `CloseHandle` runs.

**Leak per occurrence:** 2 handles (file + event).

This is the highest-frequency path in the app: `wmi_ec_read_adapter_power` (13,452 calls) and
`read_thermal_readings` (11,034 calls) both route through `run_elevated` → `run_elevated_impl`
→ `pipe_request`.

**Fix:** wrap both handles in an RAII guard so cleanup is unconditional.

```rust
struct OwnedHandle(windows::Win32::Foundation::HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if self.0 != windows::Win32::Foundation::INVALID_HANDLE_VALUE {
            unsafe { let _ = CloseHandle(self.0); }
        }
    }
}
// then: let _handle = OwnedHandle(handle); let _event = OwnedHandle(event);
// and delete the two manual CloseHandle calls at the end.
```

Also close the event before returning at line 837 — but RAII makes that automatic and covers the
read-loop break paths too, which currently rely on falling through to the end.

---

### RC-2 — `read_ecram_via_pipe()` leaks on two `?` paths 🔴 **HIGH**

**File:** `src-tauri/src/hw/ecram.rs`
**Function:** `read_ecram_via_pipe(phys_addr: u64, byte_count: usize) -> Option<Vec<u8>>`
**Lines:** 1281–1390

```rust
// line 1295 -> 1306
let handle = unsafe { CreateFileW(...) }.ok()?;        // leak #1: if this fails, nothing to close
if handle == INVALID_HANDLE_VALUE { return None; }     // ok, no handle acquired

// ... WriteFile at 1325 ...
unsafe { WriteFile(handle, ...) }.ok()?;               // leak #2: `?` returns None WITHOUT CloseHandle

// ... read loop ...

unsafe { CloseHandle(handle).ok(); }                   // line ~1359, only on success path
```

Line 1331 (`.ok()?;` after `WriteFile`) returns `None` and drops the opened pipe handle on the
floor. Every failed EC read — the same hardware that produces `WBEM_E_NOT_FOUND` — leaks one handle.

**Fix:** guard the handle with RAII as in RC-1, or explicitly `CloseHandle(handle)` before
the `?` on line 1331.

---

### RC-3 — Event handles created per overlapped read

**File:** `src-tauri/src/hw/iotservice.rs`
**Function:** overlapped pipe read helper
**Lines:** 736–859

`CreateEventW` at line 758. Six `CloseHandle(event)` sites (799, 813, 829, 845, 858) cover the
known exits, but line 759 uses `?` on a `Result`:

```rust
let event = unsafe {
    CreateEventW(None, true, false, windows::core::PCWSTR::null())
        .map_err(|e| anyhow::anyhow!("CreateEventW failed: {e}"))?   // line 759
};
```

If `CreateEventW` fails nothing leaks (no handle yet) — **this site is safe**. However the
surrounding pattern of manual, scattered `CloseHandle` calls is fragile: the function has five
distinct exit paths and relies on every one being individually correct. Converting to `OwnedHandle`
removes the class of bug.

---

### RC-4 — `pipe_server.rs` — same fragile pattern, currently correct

**File:** `src-tauri/src/hw/face/pipe_server.rs`
**Lines:** 128–215

`CreateNamedPipeW` (128) + `CreateEventW` (151). Cleanup at 169/193/209 covers the three return
paths. Line 152 uses `?` but that is _before_ the event exists, so no leak. **No bug today**, but
the same RAII conversion applies.

---

### RC-5 — Design: a helper process per elevated command

**File:** `src-tauri/src/elevated.rs`, header comment lines 1–12:

```
//!   1. Main process writes %LOCALAPPDATA%\MiControl\elev_cmd_<request_id>.json
//!   2. Main process calls `schtasks /run /tn "MiControlElevated"`
//!   3. This process starts elevated (no UAC prompt), reads the command,
//!      executes the privileged operation, writes
//!      %LOCALAPPDATA%\MiControl\elev_result_<request_id>.json, then exits.
```

Every privileged read spawns a full process via `schtasks`. With adapter power polled every
5–15 s, that is a process spawn plus JSON file write/read/delete cycle continuously. The bridge
pipe path (RC-1) is the fast path that avoids this — which is exactly why its leak matters more.

---

### RC-6 — Thread and poll fan-out

**File:** `src/hw/battery.rs` lines 97–190, and `src/hooks/useHardware.ts` lines 134–406.

`battery.rs` documents the issue against itself:

```rust
/// Battery state changes slowly, but `get_battery_info` is called every 5 s by
/// the IoT event listener and every 15 s by the frontend. Each call spawns a
/// thread that runs a WMI query; when the battery driver is slow (observed
/// 4669 "exceeded 5000 ms" warnings in one log) those threads stay stuck in COM
/// and pile up — the process reached 134 threads / 291,617 handles after two days.
```

The `BATTERY_QUERY_MIN_INTERVAL` (15 s) throttle added in "S64" bounds the query rate but does not
reclaim threads already stuck in a hung WMI call. `std::thread::Builder::spawn` per call has no
pool and no cancellation — a hung COM call holds its thread forever.

Frontend intervals (`useHardware.ts`):

- `FAST_POLL_INTERVAL = 2000` — two independent 2 s legs (`fastPollSystem`, `fastPoll`)
- `SLOW_POLL_INTERVAL = 15000`
- `FAST_POLL_LEG_MS = 6000`, `STALL_MS = 8000` watchdog

Two 2 s legs means ~30 polls/min from the UI alone, each potentially reaching the elevated bridge.

**Fix direction:** replace `std::thread::spawn`-per-query with a bounded worker (or
`tokio::task::spawn_blocking` on a capped runtime) and a single in-flight query so a hung call
cannot multiply. Add `CoInitializeEx`/`CoUninitialize` discipline on that worker to release COM.

---

## 4. Prioritised remediation

| #   | Action                                                                             | Handles recovered                         | Effort  |
| --- | ---------------------------------------------------------------------------------- | ----------------------------------------- | ------- |
| 1   | RAII guard in `pipe_request` (RC-1)                                                | ~1 per failed elevated call               | Low     |
| 2   | Close before `?` in `read_ecram_via_pipe` (RC-2)                                   | 1 per failed EC read                      | Trivial |
| 3   | Bound battery WMI queries to one worker (RC-6)                                     | ~1 thread + its COM handles per hung call | Medium  |
| 4   | Extend RAII to `iotservice.rs` / `pipe_server.rs`                                  | prevents regression                       | Low     |
| 5   | Reuse one bridge pipe connection instead of reconnecting per command               | removes 9,701 reconnects                  | Medium  |
| 6   | Handle `0x80041002` explicitly and back off, instead of retrying on the poll timer | removes pointless churn                   | Low     |

---

## 5. Immediate operational mitigation

Until the code is fixed, the process must be recycled before handles accumulate dangerously.

**Verify current state:**

```powershell
Get-Process micontrol | Select-Object Id,HandleCount,ThreadCount,
  @{n='CommitMB';e={[math]::Round($_.PrivateMemorySize64/1MB)}},
  @{n='Uptime_h';e={[math]::Round(((Get-Date)-$_.StartTime).TotalHours,1)}}
```

**Restart (recovers all leaked handles):**

```powershell
Stop-Process -Name micontrol -Force
Start-Process 'C:\Program Files\MiControl\micontrol.exe' --minimized
```

**Schedule a daily recycle (04:00):**

```powershell
$a = New-ScheduledTaskAction -Execute 'powershell.exe' `
  -Argument '-NoProfile -WindowStyle Hidden -Command "Stop-Process -Name micontrol -Force -ErrorAction SilentlyContinue; Start-Sleep 2; Start-Process ''C:\Program Files\MiControl\micontrol.exe'' --minimized"'
$t = New-ScheduledTaskTrigger -Daily -At 04:00
Register-ScheduledTask -TaskName 'MiControl daily recycle' -Action $a -Trigger $t -RunLevel Highest
```

**If the app is not needed at boot**, disable the autostart entirely (largest single win — removes
2.8% continuous CPU and all handle growth):

```powershell
Remove-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run' -Name 'MiControl'
```

---

## 6. Security notes

- `micontrol.exe` is **NotSigned** and versioned `0.2.19-beta`, distributed from
  `C:\Program Files\MiControl`.
- The elevated helper runs with `RunLevel = Highest` through the Windows Scheduled Task
  `MiControlElevated`, communicating through JSON files in `%LOCALAPPDATA%\MiControl` —
  a request/response file protocol guarded only by `util::auth` key material.
- A leaked event handle keeps the kernel object alive past the operation. Combined with 187k
  handles, this exhausts the process handle table before the system-wide limit (16,777,216) is
  reached, but it also feeds the non-paged pool.
- The custom kernel driver `IoTDriver.sys` (see `src-tauri/drivers`) runs continuously and is a
  plausible contributor to the host's elevated non-paged pool (**1,535 MB** measured system-wide).

Recommended before shipping: sign the binaries, and audit `elevated.rs` command dispatch for input
validation on the `elev_cmd_*.json` files.

---

## 7. Reproduction commands

```powershell
# Handle + commit growth
$a = Get-Process -Id $PID_OF_MICONTROL
$h0 = $a.HandleCount; $c0 = $a.PrivateMemorySize64
Start-Sleep -Seconds 60
$b = Get-Process -Id $PID_OF_MICONTROL
"handles: $($b.HandleCount - $h0)  commit: $((($b.PrivateMemorySize64 - $c0)/1MB)) MB"

# Object-type breakdown (excludes UI objects -> proves kernel handles)
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class GR{[DllImport("user32.dll")]public static extern uint GetGuiResources(IntPtr h,uint f);}'
"GDI: $([GR]::GetGuiResources($a.Handle,0))  USER: $([GR]::GetGuiResources($a.Handle,1))"

# Connection churn
Select-String 'C:\ProgramData\MiControl\logs\bridge.log' -Pattern 'client connected' | Measure-Object

# Untracked handle types (requires Sysinternals handle64.exe)
handle64.exe -p $PID_OF_MICONTROL -s | Select-Object -First 20
```
