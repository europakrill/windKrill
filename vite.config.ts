import { defineConfig } from "vite";

// Tauri expects a fixed dev port and dist output; keep in sync with
// src-tauri/crates/krill-app/tauri.conf.json.
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
