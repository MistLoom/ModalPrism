import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// 开发时把接口转发到控制台后端；构建产物由后端同源托管。
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5178,
    proxy: {
      "/api": "http://127.0.0.1:17890",
    },
  },
  build: {
    outDir: "dist",
    sourcemap: false,
  },
});
