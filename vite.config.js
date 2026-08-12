import { defineConfig } from "vite";

// Tauri drives the dev server; keep the port fixed and fail loudly if taken.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 5184,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "chrome110",
    sourcemap: false,
    chunkSizeWarningLimit: 2000,
    rollupOptions: {
      output: {
        // three's core is paid at startup whatever happens; its example loaders
        // are not, and left alone rollup hoists a shared one back into the core
        // chunk. Pin the boundary so each example stays in a chunk of its own.
        manualChunks(id) {
          if (!id.includes("node_modules/three/")) return;
          if (id.includes("/examples/")) {
            const name = id.split("/").pop().replace(/\.\w+$/, "");
            return `three-${name}`;
          }
          return "three";
        },
      },
    },
  },
});
