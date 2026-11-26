// SPDX-License-Identifier: AGPL-3.0-only
import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vitest/config";

// In development the API is the control plane on its plain-HTTP port.
const api = process.env.BN_DEV_API ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  server: { port: 5173, strictPort: true, proxy: { "/api": api } },
  test: { environment: "jsdom", setupFiles: ["./src/test-setup.ts"], globals: false },
});
