import { defineConfig } from "vite";

export default defineConfig({
  base: "/console/",
  server: {
    proxy: {
      "/console/api": "http://127.0.0.1:17880",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
