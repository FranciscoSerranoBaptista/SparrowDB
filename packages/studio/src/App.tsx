import { createSignal, onMount, Show } from "solid-js";
import "./App.css";
import { connection, saveConnection } from "./store/connection";
import { SparrowClient } from "./api/client";
import { HqlEditor } from "./views/HqlEditor";
import { SchemaBrowser } from "./views/SchemaBrowser";
import { GraphViz } from "./views/GraphViz";
import { Diagnostics } from "./views/Diagnostics";
import { Vectors } from "./views/Vectors";

type View = "hql" | "schema" | "graph" | "diagnostics" | "vectors";

export function App() {
  const [view, setView] = createSignal<View>("hql");
  const [settingsOpen, setSettingsOpen] = createSignal(!connection.baseUrl);
  const [graphData, setGraphData] = createSignal<unknown[] | undefined>(undefined);

  const [draftUrl, setDraftUrl] = createSignal(connection.baseUrl);
  const [draftKey, setDraftKey] = createSignal(connection.apiKey);

  async function saveSettings() {
    saveConnection({ baseUrl: draftUrl(), apiKey: draftKey(), connected: false });
    setSettingsOpen(false);
    try {
      await new SparrowClient(draftUrl(), draftKey()).introspect();
      saveConnection({ connected: true });
    } catch {
      // stays false
    }
  }

  onMount(async () => {
    try {
      await new SparrowClient(connection.baseUrl, connection.apiKey).introspect();
      saveConnection({ connected: true });
    } catch {
      saveConnection({ connected: false });
    }
  });

  function handleGraphResults(results: unknown[]) {
    setGraphData(results);
    setView("graph");
  }

  const navItems: { id: View; label: string }[] = [
    { id: "hql", label: "HQL Editor" },
    { id: "schema", label: "Schema" },
    { id: "graph", label: "Graph" },
    { id: "diagnostics", label: "Diagnostics" },
    { id: "vectors", label: "Vectors" },
  ];

  return (
    <div class="app">
      <aside class="sidebar">
        <div class="sidebar-logo">⬡ <span>Sparrow</span> Studio</div>
        <ul class="nav-list">
          {navItems.map((item) => (
            <li>
              <button
                class={`nav-item${view() === item.id ? " active" : ""}`}
                onClick={() => setView(item.id)}
              >
                {item.label}
              </button>
            </li>
          ))}
        </ul>
        <div class="connection-status" onClick={() => setSettingsOpen(true)}>
          <div class={`dot${connection.connected ? " connected" : ""}`} />
          {connection.baseUrl.replace("http://", "").replace("https://", "")}
        </div>
      </aside>

      <main class="main-content">
        <Show when={view() === "hql"}>
          <HqlEditor
            baseUrl={connection.baseUrl}
            apiKey={connection.apiKey}
            onGraphResults={handleGraphResults}
          />
        </Show>
        <Show when={view() === "schema"}>
          <SchemaBrowser baseUrl={connection.baseUrl} apiKey={connection.apiKey} />
        </Show>
        <Show when={view() === "graph"}>
          <GraphViz
            baseUrl={connection.baseUrl}
            apiKey={connection.apiKey}
            initialData={graphData()}
          />
        </Show>
        <Show when={view() === "diagnostics"}>
          <Diagnostics baseUrl={connection.baseUrl} apiKey={connection.apiKey} />
        </Show>
        <Show when={view() === "vectors"}>
          <Vectors baseUrl={connection.baseUrl} apiKey={connection.apiKey} />
        </Show>
      </main>

      <Show when={settingsOpen()}>
        <div class="modal-overlay" onClick={() => setSettingsOpen(false)}>
          <div class="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Connection Settings</h2>
            <div class="form-group">
              <label>Server URL</label>
              <input
                type="text"
                value={draftUrl()}
                onInput={(e) => setDraftUrl(e.currentTarget.value)}
                placeholder="http://localhost:6969"
              />
            </div>
            <div class="form-group">
              <label>API Key (leave blank if auth disabled)</label>
              <input
                type="password"
                value={draftKey()}
                onInput={(e) => setDraftKey(e.currentTarget.value)}
                placeholder=""
              />
            </div>
            <div class="modal-actions">
              <button class="btn" onClick={() => setSettingsOpen(false)}>Cancel</button>
              <button class="btn btn-primary" onClick={saveSettings}>Save</button>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
}
