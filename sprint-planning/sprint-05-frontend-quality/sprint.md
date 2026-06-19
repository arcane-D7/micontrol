# Sprint 5 — Frontend Quality & Memory Leaks

## Sprint Metadata

| Field | Value |
|-------|-------|
| **Sprint Name** | Frontend Quality & Memory Leaks |
| **Sprint Goal** | Fix interval/timeout cleanup, optimistic-update error reverts, and polling deduplication |
| **Duration Estimate** | 2 weeks (10 working days) |
| **Priority** | P1 — Frontend stability and memory hygiene. |
| **Sprint Type** | Bug Fix / Refactor |
| **Primary Owner** | Frontend engineer (React/TypeScript) |
| **Secondary Owner** | QA (memory leak reproduction) |

## Sprint Goal Statement

The frontend leaks intervals and timeouts (never cleaned up), applies optimistic UI updates that don't revert on error, and runs a redundant 500ms audio poll that duplicates the 2s hardware poll. By the end of this sprint, all intervals and timeouts are cleaned up on unmount, optimistic updates revert on command failure, and the audio poll is removed in favor of the unified hardware state. No memory leaks detectable after 30 minutes of active use.

---

## Background

Five frontend bugs were identified: (F1) `handleDetect` polling interval never cleaned up, (F2) optimistic UI updates without error revert for non-touchpad commands, (F3) redundant 500ms audio polling, (F4) `setTimeout` calls without cleanup, (F5) `touchpadDirtyUntil` ref may be overwritten. These cause memory leaks, stale UI, and wasted CPU.

---

## Tickets

### S5-001 — Clean up `handleDetect` polling interval on unmount

| Field | Value |
|-------|-------|
| **Ticket ID** | S5-001 |
| **Title** | Clear the `handleDetect` polling interval in a `useEffect` cleanup |
| **Priority** | P0 |
| **Type** | Bug |
| **Estimated Effort** | S |

#### Description

In `src/components/MainWindow.tsx` (~lines 663–678), the `handleDetect` function sets up a polling interval (`setInterval`) that is never cleared. When the component unmounts, the interval continues firing, calling `invoke` on an unmounted component — a memory leak and a source of "state update on unmounted component" warnings.

#### Affected Files and Line Ranges

- `src/components/MainWindow.tsx` — `handleDetect` (~lines 663–678).

#### Root Cause Analysis

The `setInterval` is created inside `handleDetect` but the returned interval ID is either not stored or not cleared in a `useEffect` cleanup function. React does not automatically clean up intervals; the developer must call `clearInterval` when the component unmounts.

#### Acceptance Criteria

- [ ] The interval ID is stored in a `useRef`.
- [ ] A `useEffect` with an empty dependency array (or matching the interval's lifecycle) returns a cleanup function that calls `clearInterval(intervalRef.current)`.
- [ ] If the interval is recreated conditionally, the cleanup clears the previous interval before setting a new one.
- [ ] No "state update on unmounted component" warnings in the console after unmount.
- [ ] Manual test: mount and unmount `MainWindow` 10 times; confirm no leaked intervals (check via DevTools or a counter).
- [ ] Unit test (if React Testing Library is configured): assert the interval is cleared on unmount.

#### Implementation Notes

- Prefer the `useEffect` + `setInterval` + cleanup pattern over calling `setInterval` inside an event handler, unless the interval must be triggered by user action.
- If `handleDetect` is called on demand, store the interval ID in a ref and clear it before starting a new one to avoid stacking.

#### Testing Strategy

- **Manual test**: mount/unmount cycle with interval-count logging.
- **Console check**: no unmounted-component warnings.
- **Unit test**: RTL unmount asserts cleanup called.

#### Dependencies

- None.

---

### S5-002 — Revert optimistic UI updates on command error for non-touchpad commands

| Field | Value |
|-------|-------|
| **Ticket ID** | S5-002 |
| **Title** | Add error-revert to optimistic updates in `useHardware` for all hardware commands |
| **Priority** | P1 |
| **Type** | Bug |
| **Estimated Effort** | M |

#### Description

In `src/hooks/useHardware.ts` (~lines 371–496), non-touchpad commands (brightness, volume, etc.) apply optimistic UI updates immediately but do not revert to the previous value if the backend command fails. This leaves the UI showing a state that doesn't match reality (e.g. brightness slider shows 80% but the actual brightness is still 50% because the command errored).

#### Affected Files and Line Ranges

- `src/hooks/useHardware.ts` — the command invocation handlers (~lines 371–496).

#### Root Cause Analysis

The optimistic update pattern is: (1) update local state immediately, (2) call the backend, (3) on success, confirm; on error, do nothing. Step 3's error branch is missing the revert. The touchpad path may handle this correctly (via `touchpadDirtyUntil`), but other commands do not.

#### Acceptance Criteria

- [ ] Each optimistic command handler captures the previous state value before applying the optimistic update.
- [ ] In the `.catch()` (or `Result::Err`) branch of the `invoke` call, the state is reverted to the captured previous value.
- [ ] A brief error toast/notification is shown to the user on revert (coordinate with UI patterns).
- [ ] The revert is debounced slightly (e.g. 100ms) to avoid flicker if the error returns very fast — or applied immediately if the error is slow; choose based on UX testing.
- [ ] Unit test: mock `invoke` to reject; assert state reverts to the previous value.
- [ ] Manual test: trigger a brightness change while the backend is unavailable; confirm the slider reverts and an error is shown.

#### Implementation Notes

- Implement a helper `withOptimisticUpdate<T>(currentValue, newValue, invokeFn)` that handles the capture/revert pattern to avoid duplicating logic across commands.
- Consider using a library like `react-query`'s mutation `onError` rollback if the project adopts it; otherwise a custom helper suffices.
- Ensure the revert doesn't conflict with the 2s poll (S4-002) — if the poll fires between the optimistic update and the error, the poll's value should win.

#### Testing Strategy

- **Unit test**: mock `invoke` rejection; assert revert.
- **Manual test**: disable backend, adjust sliders, confirm revert + error toast.

#### Dependencies

- None (but coordinate with S4-002's batched polling to ensure poll values don't fight the revert).

---

### S5-003 — Remove redundant 500ms audio polling

| Field | Value |
|-------|-------|
| **Ticket ID** | S5-003 |
| **Title** | Eliminate the 500ms audio poll in `AudioControl.tsx`; use the unified hardware state |
| **Priority** | P1 |
| **Type** | Bug |
| **Estimated Effort** | S |

#### Description

`src/components/AudioControl.tsx` (~lines 34–54) runs its own 500ms polling interval to fetch audio/volume state, duplicating the 2s hardware poll in `useHardware`. This wastes CPU and can cause conflicting state (the two polls may report different values at different times).

#### Affected Files and Line Ranges

- `src/components/AudioControl.tsx` — the polling `useEffect` (~lines 34–54).

#### Root Cause Analysis

`AudioControl` was likely written before `useHardware` included audio state, and its own poll was never removed. The 500ms cadence is more frequent than the 2s hardware poll, causing extra IPC and re-renders. The two sources of truth can disagree.

#### Acceptance Criteria

- [ ] The 500ms polling `useEffect` in `AudioControl.tsx` is removed.
- [ ] `AudioControl` consumes volume/mute state from `useHardware` (or the batched `get_hardware_state_batch` from S4-002).
- [ ] If real-time volume feedback is required (e.g. while dragging a slider), use an event-driven approach (Tauri event on volume change) rather than polling — but for this sprint, consuming the 2s poll is acceptable.
- [ ] No duplicate IPC calls for audio state.
- [ ] Manual test: volume control reflects the correct state; adjusting volume updates the UI.
- [ ] Performance measurement: IPC calls per second for audio drop to 0 (consumed from the batch).

#### Implementation Notes

- If the 2s cadence feels too slow for volume feedback, consider a Tauri event emitted by the backend when volume changes (event-driven, not polled). Note this as a follow-up.
- Ensure `AudioControl`'s props/context are updated to receive the hardware state.

#### Testing Strategy

- **Manual test**: volume slider and mute toggle work correctly.
- **IPC audit**: confirm no separate audio polling calls remain.

#### Dependencies

- S4-002 (batched hardware state provides the unified source).

---

### S5-004 — Clean up all `setTimeout` calls without cleanup

| Field | Value |
|-------|-------|
| **Ticket ID** | S5-004 |
| **Title** | Track and clear all `setTimeout` calls in `MainWindow.tsx` on unmount |
| **Priority** | P2 |
| **Type** | Bug |
| **Estimated Effort** | S |

#### Description

`src/components/MainWindow.tsx` has `setTimeout` calls at lines ~897, ~1114, and ~1348 that are not tracked or cleared on unmount. While `setTimeout` is less severe than `setInterval` (it fires once), if the component unmounts before the timeout fires, the callback may update state on an unmounted component or reference stale closures.

#### Affected Files and Line Ranges

- `src/components/MainWindow.tsx` — `setTimeout` at ~lines 897, 1114, 1348.

#### Root Cause Analysis

Each `setTimeout` returns an ID that should be stored and cleared in a cleanup if the component may unmount before the timeout fires. Without cleanup, the callback runs against a potentially unmounted component.

#### Acceptance Criteria

- [ ] All `setTimeout` calls in `MainWindow.tsx` store their IDs in a `useRef` (or an array of refs).
- [ ] A `useEffect` cleanup clears all pending timeouts on unmount.
- [ ] If a timeout is conditional (e.g. only set when a dialog is open), the cleanup is scoped appropriately.
- [ ] No "state update on unmounted component" warnings related to these timeouts.
- [ ] Manual test: trigger each timeout's source action, then unmount before it fires; confirm no warnings or errors.
- [ ] Code audit: grep for `setTimeout` in `MainWindow.tsx`; confirm all are tracked.

#### Implementation Notes

- For multiple timeouts, use a `useRef<number[]>` (array of IDs) and clear all in cleanup.
- Alternatively, use a custom `useTimeout` hook that encapsulates the set/clear pattern.
- Consider whether each timeout is actually needed — some may be replaceable with CSS transitions or Tauri events.

#### Testing Strategy

- **Code audit**: grep + review.
- **Manual test**: trigger-and-unmount for each timeout.

#### Dependencies

- None.

---

### S5-005 — Fix `touchpadDirtyUntil` ref overwrite race

| Field | Value |
|-------|-------|
| **Ticket ID** | S5-005 |
| **Title** | Prevent `touchpadDirtyUntil` ref from being overwritten by a stale poll refresh |
| **Priority** | P2 |
| **Type** | Bug |
| **Estimated Effort** | S |

#### Description

In `src/hooks/useHardware.ts` (~line 407), the `touchpadDirtyUntil` ref is set to mark a period during which optimistic touchpad state should be preserved (not overwritten by a poll). However, a poll refresh arriving during this window may overwrite the optimistic state, causing a flicker or revert to the pre-optimistic value.

#### Affected Files and Line Ranges

- `src/hooks/useHardware.ts` — `touchpadDirtyUntil` usage (~line 407) and the poll handler that reads it.

#### Root Cause Analysis

The `touchpadDirtyUntil` timestamp is checked, but the poll handler may not respect it correctly — either the check is missing in some code path, or the ref is reset prematurely. The intent is: "while `Date.now() < touchpadDirtyUntil`, keep the optimistic value; ignore poll updates for that field."

#### Acceptance Criteria

- [ ] Every poll handler that updates touchpad-related state checks `touchpadDirtyUntil` and skips the update if `Date.now() < touchpadDirtyUntil.current`.
- [ ] The ref is only reset (to 0 or a past timestamp) when the optimistic action completes or errors, not by a poll.
- [ ] Unit test: set `touchpadDirtyUntil` to the future; trigger a poll; assert the optimistic value is preserved.
- [ ] Manual test: perform a touchpad gesture; confirm no flicker from poll overwrites during the dirty window.

#### Implementation Notes

- Centralize the dirty-check in a helper `isTouchpadDirty()` that all poll handlers call.
- Ensure the dirty window is short enough (e.g. 500ms) that it doesn't mask real backend updates for too long.
- If the optimistic action errors, clear `touchpadDirtyUntil` so the next poll can correct the state.

#### Testing Strategy

- **Unit test**: dirty-window preservation.
- **Manual test**: gesture + poll timing.

#### Dependencies

- None (but coordinate with S5-002's revert logic).

---

## Sprint Exit Criteria

- [ ] All 5 tickets merged.
- [ ] `npm run build` passes; no TypeScript errors.
- [ ] No "state update on unmounted component" warnings in a 30-minute active-use session.
- [ ] No memory leaks detectable via Chrome DevTools heap snapshots (stable heap size after warm-up).
- [ ] No redundant IPC calls for audio state.
- [ ] Optimistic updates revert correctly on backend errors.

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Removing audio poll makes volume feedback feel sluggish | Note event-driven approach as a follow-up; 2s may be acceptable. |
| Revert logic conflicts with batched poll (S4-002) | Ensure poll respects dirty windows; coordinate timing. |
| `setTimeout` cleanup changes timing-sensitive behavior | Test each timeout's purpose; preserve intended delays. |
