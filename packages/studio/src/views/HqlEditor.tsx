import { Component, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { EditorState, Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { defaultKeymap, historyKeymap, history } from "@codemirror/commands";
import { StreamLanguage } from "@codemirror/language";
import { oneDark } from "@codemirror/theme-one-dark";
import { SparrowClient } from "../api/client";

const HQL_KEYWORDS = new Set([
  "QUERY","V","E","WHERE","TRAVERSE","RETURN","OUT","IN","ALIAS",
  "LIMIT","OFFSET","ORDER","BY","ASC","DESC","COUNT","SUM","AVG",
  "MIN","MAX","AND","OR","NOT","NULL","TRUE","FALSE",
]);

const hqlLanguage = StreamLanguage.define({
  token(stream) {
    if (stream.eatSpace()) return null;
    if (stream.match(/\/\/.*/)) return "comment";
    if (stream.match(/"([^"\\]|\\.)*"/)) return "string";
    if (stream.match(/'([^'\\]|\\.)*'/)) return "string";
    if (stream.match(/\d+(\.\d+)?/)) return "number";
    if (stream.match(/\w+/)) {
      const word = stream.current().toUpperCase();
      if (HQL_KEYWORDS.has(word)) return "keyword";
      if (stream.current()[0] === stream.current()[0].toUpperCase()) return "typeName";
      return "variableName";
    }
    stream.next();
    return null;
  },
});

const LS_HISTORY_KEY = "sparrow_hql_history";
const MAX_HISTORY = 50;

function loadHistory(): string[] {
  try {
    return JSON.parse(localStorage.getItem(LS_HISTORY_KEY) ?? "[]");
  } catch {
    return [];
  }
}

function saveToHistory(query: string) {
  const hist = loadHistory().filter((q) => q !== query);
  hist.unshift(query);
  localStorage.setItem(LS_HISTORY_KEY, JSON.stringify(hist.slice(0, MAX_HISTORY)));
}

type Tab = "table" | "json" | "graph";

export const HqlEditor: Component<{
  baseUrl: string;
  apiKey: string;
  onGraphResults: (results: unknown[]) => void;
}> = (props) => {
  let editorRef!: HTMLDivElement;
  let editorView: EditorView | undefined;
  const [result, setResult] = createSignal<unknown>(null);
  const [error, setError] = createSignal<string>("");
  const [running, setRunning] = createSignal(false);
  const [tab, setTab] = createSignal<Tab>("table");
  const [execMs, setExecMs] = createSignal<number | null>(null);
  const [historyOpen, setHistoryOpen] = createSignal(false);

  async function runQuery() {
    const query = editorView?.state.doc.toString().trim();
    if (!query) return;
    setRunning(true);
    setError("");
    const t0 = performance.now();
    try {
      const client = new SparrowClient(props.baseUrl, props.apiKey);
      const data = await client.hqlEval(query);
      setExecMs(Math.round(performance.now() - t0));
      setResult(data);
      saveToHistory(query);
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  function sendToGraph() {
    const r = result();
    if (!r) return;
    const items = Array.isArray(r) ? r : (r as { result?: unknown[] }).result ?? [];
    props.onGraphResults(items as unknown[]);
  }

  const runKeymap = keymap.of([
    { key: "Mod-Enter", run: () => { runQuery(); return true; } },
  ]);

  onMount(() => {
    const extensions: Extension[] = [
      history(),
      keymap.of([...defaultKeymap, ...historyKeymap]),
      runKeymap,
      oneDark,
      hqlLanguage,
      EditorView.lineWrapping,
    ];
    editorView = new EditorView({
      state: EditorState.create({ doc: "V | RETURN *", extensions }),
      parent: editorRef,
    });
  });

  onCleanup(() => editorView?.destroy());

  function loadHistoryItem(query: string) {
    editorView?.dispatch({
      changes: { from: 0, to: editorView.state.doc.length, insert: query },
    });
    setHistoryOpen(false);
  }

  const rows = () => {
    const r = result();
    if (!r) return [];
    return Array.isArray(r) ? r : (r as { result?: unknown[] }).result ?? [];
  };

  const columns = () => {
    const r = rows();
    if (!r.length) return [];
    return Object.keys(r[0] as object);
  };

  return (
    <div style="display:flex;flex-direction:column;height:100%;overflow:hidden">
      <div style="display:flex;align-items:center;gap:8px;padding:10px 16px;border-bottom:1px solid var(--border)">
        <span style="color:var(--text-muted);font-size:13px">Query</span>
        <div style="flex:1" />
        <Show when={historyOpen()}>
          <div style="position:absolute;right:140px;top:48px;background:var(--surface);border:1px solid var(--border);border-radius:6px;max-height:240px;overflow-y:auto;z-index:10;min-width:300px">
            <For each={loadHistory()}>
              {(q) => (
                <div
                  style="padding:8px 12px;cursor:pointer;font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;border-bottom:1px solid var(--border)"
                  onClick={() => loadHistoryItem(q)}
                >
                  {q}
                </div>
              )}
            </For>
          </div>
        </Show>
        <button class="btn" onClick={() => setHistoryOpen(!historyOpen())}>History</button>
        <button class="btn" onClick={runQuery} disabled={running()}>
          {running() ? "Running…" : "Run (⌘↵)"}
        </button>
      </div>

      <div ref={editorRef} style="height:38%;min-height:120px;overflow:auto;border-bottom:1px solid var(--border)" />

      <div style="flex:1;display:flex;flex-direction:column;overflow:hidden">
        <div style="display:flex;align-items:center;gap:0;border-bottom:1px solid var(--border);padding:0 16px">
          {(["table","json","graph"] as Tab[]).map((t) => (
            <button
              style={`background:none;border:none;border-bottom:2px solid ${tab()===t?"var(--accent)":"transparent"};padding:8px 14px;cursor:pointer;color:${tab()===t?"var(--accent)":"var(--text-muted)"};font-size:13px`}
              onClick={() => setTab(t)}
            >
              {t === "graph" ? "Graph ↗" : t.charAt(0).toUpperCase() + t.slice(1)}
            </button>
          ))}
          <div style="flex:1" />
          <span style="font-size:11px;color:var(--text-muted)">
            {rows().length} rows{execMs() != null ? ` · ${execMs()}ms` : ""}
          </span>
        </div>

        <Show when={error()}>
          <div style="padding:12px 16px;color:var(--error);font-size:13px">{error()}</div>
        </Show>

        <Show when={tab() === "table"}>
          <div style="flex:1;overflow:auto">
            <Show when={rows().length > 0}>
              <table style="width:100%;border-collapse:collapse;font-size:12px">
                <thead>
                  <tr>
                    <For each={columns()}>
                      {(col) => <th style="text-align:left;padding:6px 12px;border-bottom:1px solid var(--border);color:var(--text-muted);position:sticky;top:0;background:var(--bg)">{col}</th>}
                    </For>
                  </tr>
                </thead>
                <tbody>
                  <For each={rows()}>
                    {(row) => (
                      <tr style="border-bottom:1px solid var(--border)">
                        <For each={columns()}>
                          {(col) => (
                            <td
                              style="padding:5px 12px;cursor:pointer"
                              onClick={() => navigator.clipboard?.writeText(String((row as Record<string,unknown>)[col] ?? ""))}
                            >
                              {String((row as Record<string,unknown>)[col] ?? "")}
                            </td>
                          )}
                        </For>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </div>
        </Show>

        <Show when={tab() === "json"}>
          <pre style="flex:1;overflow:auto;padding:12px 16px;font-size:12px;color:var(--text)">
            {JSON.stringify(result(), null, 2)}
          </pre>
        </Show>

        <Show when={tab() === "graph"}>
          <div style="padding:16px;display:flex;align-items:center;gap:12px">
            <button class="btn btn-primary" onClick={sendToGraph}>Send to Graph View</button>
            <span style="font-size:12px;color:var(--text-muted)">{rows().length} items will be visualised</span>
          </div>
        </Show>
      </div>
    </div>
  );
};
