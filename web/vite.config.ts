import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"

// Single-bundle build: web/dist is a committed release artifact embedded into
// the talon binary (rust-embed, `web-ui` feature). One hashed JS + one CSS
// file keeps the committed diff small. Rebuild with: bun run build
export default defineConfig({
  plugins: [react()],
  base: "/ui/",
  build: {
    cssCodeSplit: false,
    rollupOptions: {
      output: {
        manualChunks: undefined,
        entryFileNames: "assets/app.js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: "assets/[name][extname]",
      },
    },
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:7777",
    },
  },
})
