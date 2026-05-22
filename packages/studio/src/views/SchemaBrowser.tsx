import { Component, createResource, For, Show } from "solid-js";
import { SparrowClient, SchemaNode, SchemaEdge } from "../api/client";

const NodeCard: Component<{ node: SchemaNode }> = (props) => (
  <div style="background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:14px;min-width:180px">
    <div style="color:var(--accent);font-weight:600;margin-bottom:8px">{props.node.name}</div>
    <For each={Object.entries(props.node.properties)}>
      {([key, type]) => (
        <div style="font-size:12px;color:var(--text-muted);margin-bottom:2px">
          <span style="color:var(--text)">{key}</span>: {type}
        </div>
      )}
    </For>
  </div>
);

const EdgeCard: Component<{ edge: SchemaEdge }> = (props) => (
  <div style="background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:14px;min-width:200px">
    <div style="color:var(--accent-orange);font-weight:600;margin-bottom:4px">{props.edge.name}</div>
    <div style="font-size:12px;color:var(--text-muted);margin-bottom:8px">
      {props.edge.from} → {props.edge.to}
    </div>
    <For each={Object.entries(props.edge.properties)}>
      {([key, type]) => (
        <div style="font-size:12px;color:var(--text-muted);margin-bottom:2px">
          <span style="color:var(--text)">{key}</span>: {type}
        </div>
      )}
    </For>
  </div>
);

export const SchemaBrowser: Component<{ baseUrl: string; apiKey: string }> = (props) => {
  const [schema] = createResource(
    () => ({ baseUrl: props.baseUrl, apiKey: props.apiKey }),
    async ({ baseUrl, apiKey }) => new SparrowClient(baseUrl, apiKey).introspect()
  );

  return (
    <div class="view">
      <Show when={schema.loading}>
        <p style="color:var(--text-muted)">Loading schema…</p>
      </Show>
      <Show when={schema.error}>
        <p style="color:var(--error)">Error: {String(schema.error)}</p>
      </Show>
      <Show when={schema()}>
        {(s) => (
          <>
            <section style="margin-bottom:28px">
              <h2 style="font-size:14px;margin-bottom:12px;color:var(--text-muted)">
                Node Types ({s().nodes.length})
              </h2>
              <div style="display:flex;flex-wrap:wrap;gap:12px">
                <For each={s().nodes}>{(node) => <NodeCard node={node} />}</For>
              </div>
            </section>
            <section>
              <h2 style="font-size:14px;margin-bottom:12px;color:var(--text-muted)">
                Edge Types ({s().edges.length})
              </h2>
              <div style="display:flex;flex-wrap:wrap;gap:12px">
                <For each={s().edges}>{(edge) => <EdgeCard edge={edge} />}</For>
              </div>
            </section>
          </>
        )}
      </Show>
    </div>
  );
};
