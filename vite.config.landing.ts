import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'path';

/**
 * Vite config for the miControl landing page (GitHub Pages + custom domain).
 * Entry: landing.html
 * Output: dist-landing/
 * Base: ./ (relative paths so the same build works both on
 * arcane-d7.github.io/micontrol/ and micontrol.mfreitas.dev/)
 *
 * Usage: npx vite build --config vite.config.landing.ts
 */
export default defineConfig({
  plugins: [react()],
  base: './',
  resolve: {
    alias: {
      // Use mock Tauri API so the live app preview works in the landing page
      '@tauri-apps/api/core': resolve(__dirname, 'src/mocks/tauri-api.ts'),
    },
  },
  define: {
    // Prevents Tauri IPC initialisation from crashing in plain browser
    __TAURI_INTERNALS__: 'undefined',
  },
  build: {
    outDir: 'dist-landing',
    rollupOptions: {
      input: resolve(__dirname, 'landing.html'),
      output: {
        manualChunks: {
          'react-vendor': ['react', 'react-dom'],
          'three-vendor': ['three', '@react-three/fiber', '@react-three/drei'],
          'gsap-vendor': ['gsap', '@gsap/react', 'lenis'],
        },
      },
    },
  },
});
