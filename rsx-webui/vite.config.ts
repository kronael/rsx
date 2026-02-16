import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  base: "/trade/",
  plugins: [react()],
  server: {
    port: 3000,
    proxy: {
      "/ws/private": {
        target: "ws://localhost:8080",
        ws: true,
      },
      "/ws/public": {
        target: "ws://localhost:8081",
        ws: true,
      },
      "/v1": {
        target: "http://localhost:8080",
      },
    },
  },
});
