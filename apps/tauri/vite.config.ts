/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri-compatible Vite config (https://v2.tauri.app/start/frontend/vite/).
// Keep the dev port in step with `build.devUrl` in src-tauri/tauri.conf.json. Not 1420 (the
// Tauri template default): Windows reserves ranges such as 1333-1432 and then refuses to bind.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 5174 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // WebView2 (Chromium) on Windows, WebKit on macOS/Linux.
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome111" : "safari16",
    minify: process.env.TAURI_ENV_DEBUG ? false : undefined,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    css: false,
    testTimeout: 15000,
  },
});
