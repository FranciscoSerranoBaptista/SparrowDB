import { Component, createSignal } from "solid-js";
import { SparrowClient } from "../api/client";

export const Vectors: Component<{ baseUrl: string; apiKey: string }> = (props) => {
  const [deleteId, setDeleteId] = createSignal("");
  const [status, setStatus] = createSignal("");
  const [error, setError] = createSignal("");

  function client() {
    return new SparrowClient(props.baseUrl, props.apiKey);
  }

  async function run(action: () => Promise<void>, successMsg: string) {
    setStatus("");
    setError("");
    try {
      await action();
      setStatus(successMsg);
    } catch (e) {
      setError(String(e));
    }
  }

  async function softDelete() {
    const id = deleteId().trim();
    if (!id) return;
    await run(() => client().vectorSoftDelete(id), `Soft-deleted vector ${id}`);
  }

  async function hardDelete() {
    const id = deleteId().trim();
    if (!id || !confirm(`Hard-delete vector ${id}? This cannot be undone.`)) return;
    await run(() => client().vectorHardDelete(id), `Hard-deleted vector ${id}`);
  }

  async function purge() {
    if (!confirm("Purge all soft-deleted vectors?")) return;
    await run(() => client().purgeSoftDeleted(), "Soft-deleted vectors purged");
  }

  async function rebuild() {
    if (!confirm("Rebuild the entire HNSW index? This may take a moment.")) return;
    await run(() => client().rebuildVectorIndex(), "Vector index rebuilt");
  }

  return (
    <div class="view">
      <h1 style="font-size:15px;margin-bottom:20px">Vector Index</h1>
      {status() && <p class="status-ok">{status()}</p>}
      {error() && <p class="status-err">{error()}</p>}

      <div style="display:flex;flex-direction:column;gap:16px;max-width:480px;margin-top:12px">
        <div style="background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:16px">
          <h2 style="font-size:13px;margin-bottom:10px">Delete by ID</h2>
          <input
            style="width:100%;background:var(--bg);border:1px solid var(--border);border-radius:6px;padding:7px 10px;color:var(--text);font-size:13px;margin-bottom:10px"
            placeholder="UUID or u128 string"
            value={deleteId()}
            onInput={(e) => setDeleteId(e.currentTarget.value)}
          />
          <div style="display:flex;gap:8px">
            <button class="btn" onClick={softDelete}>Soft Delete</button>
            <button class="btn btn-danger" onClick={hardDelete}>Hard Delete</button>
          </div>
        </div>

        <div style="background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:16px">
          <h2 style="font-size:13px;margin-bottom:6px">Purge Soft-Deleted</h2>
          <p style="font-size:12px;color:var(--text-muted);margin-bottom:10px">
            Removes all soft-deleted vector entries permanently.
          </p>
          <button class="btn btn-danger" onClick={purge}>Purge Soft-Deleted</button>
        </div>

        <div style="background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:16px">
          <h2 style="font-size:13px;margin-bottom:6px">Rebuild Index</h2>
          <p style="font-size:12px;color:var(--text-muted);margin-bottom:10px">
            Rebuilds the entire HNSW index. This may take a moment for large graphs.
          </p>
          <button class="btn btn-danger" onClick={rebuild}>Rebuild Index</button>
        </div>
      </div>
    </div>
  );
};
