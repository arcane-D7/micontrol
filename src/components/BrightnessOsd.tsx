import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow, PhysicalPosition, primaryMonitor } from '@tauri-apps/api/window';

// ── Brightness OSD overlay ────────────────────────────────────────────────────
// Renders inside the dedicated `brightness-osd` Tauri window (260×88 px,
// always-on-top, transparent, no decorations).
// Listens for `gesture:brightness_changed` events emitted by the Rust gesture
// thread, displays a pill-shaped glass card, then hides itself after 1.5 s.
// ─────────────────────────────────────────────────────────────────────────────

export default function BrightnessOsd() {
  const [level, setLevel] = useState(50);
  const [visible, setVisible] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    // Position the window at bottom-centre of the primary monitor on first mount.
    (async () => {
      try {
        const win = getCurrentWindow();
        const monitor = await primaryMonitor();
        if (monitor) {
          const winSize = await win.outerSize();
          await win.setPosition(
            new PhysicalPosition(
              monitor.position.x + Math.floor((monitor.size.width - winSize.width) / 2),
              monitor.position.y + monitor.size.height - winSize.height - 64,
            ),
          );
        }
      } catch {
        /* non-fatal */
      }
    })();

    const unlisten = listen<number>('gesture:brightness_changed', (event) => {
      setLevel(Math.round(event.payload));
      setVisible(true);

      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        setVisible(false);
      }, 1500);
    });

    return () => {
      unlisten.then((f) => f());
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  const sunPath =
    'M12 7a5 5 0 1 1 0 10A5 5 0 0 1 12 7zm0-4v2m0 14v2M4.22 4.22l1.42 1.42' +
    'm12.72 12.72 1.42 1.42M2 12h2m16 0h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42';

  return (
    <div
      data-tauri-drag-region
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: 'transparent',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '12px',
          padding: '14px 20px',
          borderRadius: '20px',
          background: 'oklch(14% 0.02 260 / 0.82)',
          backdropFilter: 'blur(20px) saturate(1.4)',
          WebkitBackdropFilter: 'blur(20px) saturate(1.4)',
          border: '1px solid oklch(100% 0 0 / 0.12)',
          boxShadow: '0 8px 32px oklch(0% 0 0 / 0.4)',
          opacity: visible ? 1 : 0,
          transform: `translateY(${visible ? 0 : 6}px)`,
          transition: 'opacity 0.18s ease, transform 0.18s ease',
          minWidth: '220px',
        }}
      >
        {/* Sun icon */}
        <svg
          width="22"
          height="22"
          viewBox="0 0 24 24"
          fill="none"
          stroke="oklch(88% 0.12 75)"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          style={{ flexShrink: 0 }}
        >
          <path d={sunPath} />
        </svg>

        {/* Bar + percentage */}
        <div style={{ flex: 1 }}>
          <div
            style={{
              height: '6px',
              borderRadius: '3px',
              background: 'oklch(100% 0 0 / 0.15)',
              overflow: 'hidden',
              marginBottom: '6px',
            }}
          >
            <div
              style={{
                height: '100%',
                width: `${level}%`,
                borderRadius: '3px',
                background: 'linear-gradient(90deg, oklch(75% 0.15 75), oklch(85% 0.18 60))',
                transition: 'width 0.15s ease',
              }}
            />
          </div>
          <div
            style={{
              fontSize: '12px',
              fontWeight: 500,
              color: 'oklch(88% 0.008 260)',
              letterSpacing: '0.01em',
              fontFamily: "'Outfit', system-ui, sans-serif",
              textAlign: 'right',
            }}
          >
            {level}%
          </div>
        </div>
      </div>
    </div>
  );
}
