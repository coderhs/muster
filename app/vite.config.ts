import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Port 1420 is what tauri.conf.json's devUrl points at.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  build: {
    target: "safari15",
    chunkSizeWarningLimit: 600,
  },
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});
