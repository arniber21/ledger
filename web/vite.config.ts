import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const apiOrigin = process.env.LEDGER_API_ORIGIN ?? "http://127.0.0.1:8080";
const uiPort = Number(process.env.LEDGER_UI_PORT ?? "5173");

export default defineConfig({
  plugins: [react()],
  server: {
    port: uiPort,
    proxy: {
      "/api": apiOrigin,
      "/v1": apiOrigin,
    },
    watch: {
      ignored: ["**/e2e/report/**", "**/e2e/test-results/**"],
    },
  },
});
