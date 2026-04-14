import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import wasm from "vite-plugin-wasm";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [sveltekit(), wasm()],

  // Tauri targets a modern webview — bump target so top-level await
  // (used by capsaicin + novnc WASM bootstrap) transpiles cleanly.
  build: {
    target: ["es2022", "chrome111", "edge111", "firefox115", "safari16"],
  },

  optimizeDeps: {
    exclude: ["crytter-wasm"],
  },

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
