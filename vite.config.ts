import { defineConfig } from "vite";

// Tauri expects a fixed dev port and dist output; keep in sync with
// src-tauri/tauri.conf.json when the desktop shell is wired (M2).
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    target: "chrome110",
  },
});
