import { defineConfig, loadEnv } from "vite";
import solidPlugin from "vite-plugin-solid";

const PROXY_PATHS = [
  "/v1",
  "/__hql_runtime_eval",
  "/introspect",
  "/diagnostics",
  "/hnsw_health",
  "/hnsw_integrity",
  "/vector_soft_delete",
  "/vector_hard_delete",
  "/purge_soft_deleted",
  "/rebuild_vector_index",
];

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const apiKey = env.SPARROW_API_KEY ?? "";
  const target = env.SPARROW_URL ?? "http://localhost:6969";

  const proxyEntry = (path: string) => ({
    target,
    changeOrigin: true,
    configure: (proxy: import("vite").HttpProxy.Server) => {
      proxy.on("proxyReq", (proxyReq) => {
        if (apiKey) proxyReq.setHeader("x-api-key", apiKey);
      });
    },
  });

  return {
    plugins: [solidPlugin()],
    base: "/__studio/",
    server: {
      port: 5173,
      proxy: Object.fromEntries(PROXY_PATHS.map((p) => [p, proxyEntry(p)])),
    },
    build: {
      outDir: "dist",
      emptyOutDir: true,
    },
    test: {
      environment: "jsdom",
    },
  };
});
