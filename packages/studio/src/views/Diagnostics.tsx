import { Component, createSignal, onCleanup } from "solid-js";
import { SparrowClient, DiagnosticsResponse, HnswHealthResponse, VectorStats } from "../api/client";

export const Diagnostics: Component<{ baseUrl: string; apiKey: string }> = (props) => {
  const [diag, setDiag] = createSignal<DiagnosticsResponse | null>(null);
  const [hnsw, setHnsw] = createSignal<HnswHealthResponse | null>(null);
  const [integrityMsg, setIntegrityMsg] = createSignal("");
  const [error, setError] = createSignal("");
  const [autoRefresh, setAutoRefresh] = createSignal(false);
  const [refreshing, setRefreshing] = createSignal(false);
  let timer: ReturnType<typeof setInterval> | undefined;

  async function fetchAll() {
    setRefreshing(true);
    setError("");
    try {
      const client = new SparrowClient(props.baseUrl, props.apiKey);
      const [d, h] = await Promise.all([client.diagnostics(), client.hnswHealth()]);
      setDiag(d);
      setHnsw(h);
    } catch (e) {
      setError(String(e));
    } finally {
      setRefreshing(false);
    }
  }

  async function checkIntegrity() {
    setIntegrityMsg("Checking…");
    try {
      const client = new SparrowClient(props.baseUrl, props.apiKey);
      const result = await client.hnswIntegrity();
      setIntegrityMsg(result.ok ? `OK: ${result.details}` : `Issues: ${result.details}`);
    } catch (e) {
      setIntegrityMsg(String(e));
    }
  }

  fetchAll();

  function toggleAutoRefresh() {
    const next = !autoRefresh();
    setAutoRefresh(next);
    if (next) {
      timer = setInterval(fetchAll, 10_000);
    } else {
      clearInterval(timer);
    }
  }

  onCleanup(() => clearInterval(timer));

  function fmt(n: number | undefined) {
    return n != null ? n.toLocaleString() : "—";
  }

  function fmtBytes(b: number | undefined) {
    if (b == null) return "—";
    if (b > 1_073_741_824) return `${(b / 1_073_741_824).toFixed(2)} GB`;
    if (b > 1_048_576) return `${(b / 1_048_576).toFixed(2)} MB`;
    return `${(b / 1024).toFixed(1)} KB`;
  }

  return (
    <div class="view">
      <div style="display:flex;align-items:center;gap:12px;margin-bottom:20px">
        <h1 style="font-size:15px">Diagnostics</h1>
        <button class="btn" onClick={fetchAll} disabled={refreshing()}>
          {refreshing() ? "Refreshing…" : "Refresh"}
        </button>
        <button class={`btn${autoRefresh() ? " btn-primary" : ""}`} onClick={toggleAutoRefresh}>
          Auto-refresh {autoRefresh() ? "ON" : "OFF"}
        </button>
      </div>
      {error() && <p style="color:var(--error);margin-bottom:12px">{error()}</p>}
      <div style="display:grid;grid-template-columns:1fr 1fr;gap:16px;max-width:700px">
        <div style="background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:16px">
          <h2 style="font-size:13px;color:var(--text-muted);margin-bottom:12px">System Stats</h2>
          <div style="display:grid;grid-template-columns:1fr 1fr;gap:10px">
            {[
              ["Nodes", fmt(diag()?.nodes)],
              ["Edges", fmt(diag()?.edges)],
              ["Vectors (active)", fmt(diag()?.vectors?.active)],
              ["Vectors (total)", fmt(diag()?.vectors?.total)],
            ].map(([label, value]) => (
              <div>
                <div style="font-size:11px;color:var(--text-muted)">{label}</div>
                <div style="font-size:18px;font-weight:600">{value}</div>
              </div>
            ))}
          </div>
        </div>
        <div style="background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:16px">
          <h2 style="font-size:13px;color:var(--text-muted);margin-bottom:12px">HNSW Health</h2>
          <div style="display:grid;grid-template-columns:1fr 1fr;gap:10px;margin-bottom:12px">
            {[
              ["Status", hnsw()?.status ?? "—"],
              ["Vectors", fmt(hnsw()?.vector_count)],
              ["Soft-deleted", fmt(hnsw()?.soft_deleted_count)],
            ].map(([label, value]) => (
              <div>
                <div style="font-size:11px;color:var(--text-muted)">{label}</div>
                <div
                  style={`font-size:16px;font-weight:600;color:${label === "Status" && value === "healthy" ? "var(--success)" : "var(--text)"}`}
                >
                  {value}
                </div>
              </div>
            ))}
          </div>
          <button class="btn" onClick={checkIntegrity}>Check Integrity</button>
          {integrityMsg() && (
            <p style="font-size:12px;color:var(--text-muted);margin-top:8px">{integrityMsg()}</p>
          )}
        </div>
      </div>
    </div>
  );
};
