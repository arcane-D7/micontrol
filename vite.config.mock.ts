import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

// Mock mode: replaces @tauri-apps/api/core with src/mocks/tauri-api.ts
// Usage: npm run dev:mock
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  resolve: {
    alias: {
      "@tauri-apps/api/core": path.resolve(
        __dirname,
        "src/mocks/tauri-api.ts"
      ),
    },
  },
  define: {
    // Prevents Tauri IPC initialisation from crashing in plain browser
    "__TAURI_INTERNALS__": "undefined",
  },
  server: {
    port: 1421,
    strictPort: false,
    open: true,
  },
});
