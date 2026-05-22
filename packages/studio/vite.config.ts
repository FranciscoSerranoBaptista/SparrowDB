import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";

export default defineConfig({
  plugins: [solidPlugin()],
  base: "/__studio/",
  server: {
    port: 5173,
    proxy: {
      "/v1": "http://localhost:6969",
      "/__hql_runtime_eval": "http://localhost:6969",
      "/introspect": "http://localhost:6969",
      "/diagnostics": "http://localhost:6969",
      "/hnsw_health": "http://localhost:6969",
      "/hnsw_integrity": "http://localhost:6969",
      "/vector_soft_delete": "http://localhost:6969",
      "/vector_hard_delete": "http://localhost:6969",
      "/purge_soft_deleted": "http://localhost:6969",
      "/rebuild_vector_index": "http://localhost:6969",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  test: {
    environment: "jsdom",
  },
});
