import { defineConfig } from "vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
// @ts-expect-error type error without @types/node package
import { readFileSync } from "node:fs";
const host = process.env.TAURI_DEV_HOST;

// Dev CSP (PRD R1, docs/SPIKE-csp-findings.md §3): with devUrl set, Tauri does
// NOT inject a CSP in dev, so the dev server must send it itself. Single source
// of truth is app.security.csp in src-tauri/tauri.conf.json — edit the policy
// there, never here; this only adds ws://localhost:1420 (Vite HMR) to connect-src.
const prodCsp = JSON.parse(
  readFileSync(new URL("./src-tauri/tauri.conf.json", import.meta.url), "utf8"),
).app.security.csp as string;
const devCsp = prodCsp.replace(
  "connect-src 'self'",
  "connect-src 'self' ws://localhost:1420",
);

// https://vite.dev/config/
export default defineConfig(() => ({

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    headers: { "Content-Security-Policy": devCsp },
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
