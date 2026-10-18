import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Dashboard for the 111 node. Built to a static dist/ that 111d embeds and serves same-origin.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "./",
  build: {
    target: "es2020",
    sourcemap: false,
    reportCompressedSize: true,
  },
  server: {
    // The Licenses page imports os/LICENSE.md and os/THIRD-PARTY-NOTICES.md (`?raw`), two levels above this project.
    fs: { allow: [fileURLToPath(new URL("../..", import.meta.url))] },
    // Dev against a running 111d: everything under /api is same-origin in prod.
    proxy: { "/api": { target: "http://127.0.0.1:11180", changeOrigin: false } },
  },
  test: {
    environment: "node",
    include: ["test/**/*.test.ts", "src/**/*.test.ts"],
  },
});
