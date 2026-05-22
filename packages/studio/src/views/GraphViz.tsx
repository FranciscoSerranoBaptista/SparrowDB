import { Component, createSignal, onMount, onCleanup, createEffect } from "solid-js";
import cytoscape, { Core, ElementDefinition } from "cytoscape";
// @ts-expect-error no bundled types for layout
import coseBilkent from "cytoscape-cose-bilkent";
import { SparrowClient } from "../api/client";

cytoscape.use(coseBilkent);

function parseResults(items: unknown[]): ElementDefinition[] {
  const elements: ElementDefinition[] = [];
  for (const item of items) {
    const obj = item as Record<string, unknown>;
    if (obj.from !== undefined && obj.to !== undefined) {
      elements.push({
        data: {
          id: String(obj.id ?? `${obj.from}->${obj.to}`),
          source: String(obj.from),
          target: String(obj.to),
          label: String(obj.type ?? ""),
        },
      });
    } else if (obj.id !== undefined) {
      elements.push({
        data: {
          id: String(obj.id),
          label: String(obj.type ?? obj.id),
          ...obj,
        },
      });
    }
  }
  return elements;
}

export const GraphViz: Component<{
  baseUrl: string;
  apiKey: string;
  initialData?: unknown[];
}> = (props) => {
  let canvasRef!: HTMLDivElement;
  let cy: Core | undefined;
  const [query, setQuery] = createSignal("QUERY getAll() =>\n    result <- N<People>\nRETURN result");
  const [running, setRunning] = createSignal(false);
  const [error, setError] = createSignal("");
  const [selected, setSelected] = createSignal<Record<string, unknown> | null>(null);

  function initCy(elements: ElementDefinition[]) {
    cy?.destroy();
    cy = cytoscape({
      container: canvasRef,
      elements,
      style: [
        {
          selector: "node",
          style: {
            label: "data(label)",
            "background-color": "#58a6ff",
            color: "#e6edf3",
            "font-size": 11,
            "text-valign": "bottom",
            "text-margin-y": 4,
          },
        },
        {
          selector: "edge",
          style: {
            label: "data(label)",
            "line-color": "#30363d",
            "target-arrow-color": "#30363d",
            "target-arrow-shape": "triangle",
            "curve-style": "bezier",
            color: "#8b949e",
            "font-size": 10,
          },
        },
        {
          selector: "node:selected",
          style: { "background-color": "#f0883e" },
        },
      ],
      layout: { name: "cose-bilkent" } as never,
      // @ts-expect-error backgroundColor not in CytoscapeOptions typedef
      backgroundColor: "#0d1117",
    });

    cy.on("tap", "node", (e) => {
      setSelected(e.target.data());
    });
    cy.on("tap", (e) => {
      if (e.target === cy) setSelected(null);
    });
  }

  async function runQuery() {
    const q = query().trim();
    if (!q) return;
    setRunning(true);
    setError("");
    try {
      const client = new SparrowClient(props.baseUrl, props.apiKey);
      const data = await client.hqlEval(q);
      const items = Array.isArray(data) ? data : (data as { result?: unknown[] }).result ?? [];
      initCy(parseResults(items));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  onMount(() => {
    initCy(props.initialData ? parseResults(props.initialData) : []);
  });

  createEffect(() => {
    if (props.initialData && props.initialData.length > 0) {
      initCy(parseResults(props.initialData));
    }
  });

  onCleanup(() => cy?.destroy());

  return (
    <div style="display:flex;flex-direction:column;height:100%;overflow:hidden">
      <div style="display:flex;gap:8px;align-items:center;padding:10px 16px;border-bottom:1px solid var(--border)">
        <input
          style="flex:1;background:var(--bg);border:1px solid var(--border);border-radius:6px;padding:7px 10px;color:var(--text);font-size:13px"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          placeholder="QUERY getAll() => result <- N<People> RETURN result"
          onKeyDown={(e) => e.key === "Enter" && runQuery()}
        />
        <button class="btn" onClick={runQuery} disabled={running()}>
          {running() ? "Running…" : "Run"}
        </button>
        <button class="btn" onClick={() => cy?.fit()}>Fit</button>
        <button class="btn" onClick={() => cy?.zoom(cy.zoom() * 1.2)}>+</button>
        <button class="btn" onClick={() => cy?.zoom(cy.zoom() * 0.8)}>−</button>
        <button class="btn" onClick={() => { cy?.layout({ name: "cose-bilkent" } as never).run(); }}>Reset</button>
      </div>
      {error() && <div style="padding:8px 16px;color:var(--error);font-size:12px">{error()}</div>}
      <div style="position:relative;flex:1">
        <div ref={canvasRef} style="width:100%;height:100%;background:#0d1117" />
        {selected() && (
          <div style="position:absolute;top:12px;right:12px;background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:14px;max-width:260px;overflow:auto;max-height:50%">
            <div style="font-size:13px;font-weight:600;margin-bottom:8px">Node: {String(selected()!.id)}</div>
            {Object.entries(selected()!).filter(([k]) => k !== "id").map(([k, v]) => (
              <div style="font-size:12px;color:var(--text-muted);margin-bottom:3px">
                <span style="color:var(--text)">{k}</span>: {String(v)}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
