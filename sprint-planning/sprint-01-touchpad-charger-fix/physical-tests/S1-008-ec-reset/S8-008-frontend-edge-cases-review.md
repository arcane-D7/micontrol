# S8-008 — Frontend Edge Cases Review

## Files Reviewed

### Hooks
- `src/hooks/useHardware.ts` — 20 `invoke()` calls, all wrapped in try/catch
- `src/hooks/useSettings.ts` — 5 `invoke()` calls, all wrapped in try/catch; 2 `JSON.parse()` calls, both wrapped in try/catch
- `src/hooks/useAnalysisLogger.ts` — 4 `JSON.parse()` calls, all wrapped in try/catch

### Components
- `src/components/EcrDebugPanel.tsx` — 3 `invoke()` calls, all in try/catch
- `src/components/StartupManager.tsx` — 1 `invoke()` call, in try/catch
- `src/components/WiFiManager.tsx` — 2 `invoke()` calls, all in try/catch; loading state present
- `src/components/BrightnessOsd.tsx` — 0 `invoke()` calls (uses `listen`); fixed floating promises
- `src/components/IotDeviceCard.tsx` — 1 `invoke()` call, in try/catch; loading state present
- `src/components/AiAnalysis.tsx` — 0 `invoke()` calls; 2 `JSON.parse()` calls, both in try/catch; loading state present
- `src/components/HardwareDiscovery.tsx` — 0 `invoke()` calls (uses callback props); loading state present

### Pages
- `src/pages/MainWindow.tsx` — 2 `invoke()` calls (dynamic import), both in try/catch; loading states present
- `src/pages/TrayPopup.tsx` — 3 `invoke()` calls, all with `void` prefix (fire-and-forget)

## Issues Found and Fixed

### Floating Promises (no-floating-promises)
| File | Issue | Fix |
|------|-------|-----|
| `src/hooks/useHardware.ts` | `writeAiPerfLog`, `openAiLogsDir`, `readAiPerfLogs`, `getProcessList`, `installDriver`, `getEcramMap`, `getIotRegionHex`, `writeIotHex`, `readEcramRaw`, `isElevated`, `relaunchAsAdmin` — missing try/catch | Added try/catch with console.error |
| `src/components/BrightnessOsd.tsx` | IIFE and `unlisten.then()` floating promises | Added `void` prefix |
| `src/components/IotDeviceCard.tsx` | `loadInfo()` call in useEffect | Added `void` prefix |
| `src/components/WiFiManager.tsx` | `loadData()` calls in useEffect, handleConnect, handleDisconnect, handleRefresh, onClick | Added `void` prefix |
| `src/hooks/useSettings.ts` | `saveSettings()` call in `updateKey`, IIFE in useEffect | Added `void` prefix |
| `src/pages/MainWindow.tsx` | `getTelemetryConsent()` in SettingsTab, `import()` in KeyboardTab useEffect | Added `void` prefix + .catch |

### Unhandled invoke rejections
| File | Issue | Fix |
|------|-------|-----|
| `src/hooks/useHardware.ts` | `writeAiPerfLog`, `openAiLogsDir`, `readAiPerfLogs`, `getProcessList`, `installDriver`, `getEcramMap`, `getIotRegionHex`, `writeIotHex`, `readEcramRaw`, `isElevated`, `relaunchAsAdmin` | Added try/catch |
| `src/pages/MainWindow.tsx` | `handleDetect` — `start_key_detect` and interval `get_detected_key` | Added try/catch |

### ESLint Rule Changes
Added to `eslint.config.js`:
- `@typescript-eslint/no-floating-promises: 'warn'` — catches unhandled promise rejections
- Added `projectService: true` and `tsconfigRootDir` to `parserOptions` (required by no-floating-promises)

### Loading States
- `WiFiManager.tsx` — shows loading state while fetching networks
- `IotDeviceCard.tsx` — shows loading state while fetching device info
- `EcrDebugPanel.tsx` — shows loading state during EC operations
- `HardwareDiscovery.tsx` — shows scanning state during discovery
- `AiAnalysis.tsx` — shows analyzing state during AI analysis

## Health Check Results
- `npx tsc --noEmit` — ✅ Pass (0 errors)
- `npm run build` — ✅ Pass (75 modules, built in 1.4s)
- `npm run lint` — ✅ Pass (0 errors, 9 pre-existing warnings)
- `npm run format:check` — ✅ Pass (all files use Prettier code style)

## Remaining Pre-existing Warnings (not related to this ticket)
1. `App.tsx` — conditional hook calls (react-hooks/rules-of-hooks) — architectural pattern
2. `PerformanceModeSelector.test.tsx` — unused import
3. `DisplaySettings.tsx` — unescaped entities
4. `useSettings.ts` — unused `_` variable, missing useEffect dep
