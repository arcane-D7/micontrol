import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './styles/globals.css';

// P3GLEG tauri-plugin-mcp guest bindings — wires the webview side of the
// MCP server (needed by execute_js / query_page / click / read_text /
// type_text / navigate etc.).
//
// Runs in ALL builds (dev AND the installed app) so the MCP toolset works
// identically in the final version when the user enables Settings →
// "MCP Integration". The Rust-side socket server only *starts* when that
// toggle is ON (registry flag), so the listeners below are inert (no-op)
// while the toggle is off — no security exposure.
//
// Failed/blocked loads are non-fatal (MCP simply not available).
void (async () => {
  try {
    const { setupPluginListeners } = await import('tauri-plugin-mcp');
    await setupPluginListeners();
  } catch {
    /* MCP unavailable in this build */
  }
})();

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
