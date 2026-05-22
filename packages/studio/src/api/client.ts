export type SchemaNode = { name: string; properties: Record<string, string> };
export type SchemaEdge = { name: string; from: string; to: string; properties: Record<string, string> };
export type SchemaResponse = { nodes: SchemaNode[]; edges: SchemaEdge[] };
export type VectorStats = { total: number; active: number; soft_deleted: number; hnsw_edges: number; entry_point_present: boolean };
export type DiagnosticsResponse = { nodes: number; edges: number; vectors: VectorStats };
export type HnswHealthResponse = { status: string; vector_count: number; soft_deleted_count: number };
export type HnswIntegrityResponse = { ok: boolean; details: string };

export function parseSchema(raw: string): SchemaResponse {
  const nodes: SchemaNode[] = [];
  const edges: SchemaEdge[] = [];
  const nodeRe = /N::(\w+)\s*\{([^}]*)\}/g;
  const edgeRe = /E::(\w+)\s*\{([^}]*)\}/g;
  let m: RegExpExecArray | null;

  while ((m = nodeRe.exec(raw)) !== null) {
    const props = parseProps(m[2]);
    nodes.push({ name: m[1], properties: props });
  }
  while ((m = edgeRe.exec(raw)) !== null) {
    const props = parseProps(m[2]);
    const from = props["from"] ?? "";
    const to = props["to"] ?? "";
    const rest = Object.fromEntries(
      Object.entries(props).filter(([k]) => k !== "from" && k !== "to")
    );
    edges.push({ name: m[1], from, to, properties: rest });
  }
  return { nodes, edges };
}

function parseProps(block: string): Record<string, string> {
  const result: Record<string, string> = {};
  for (const line of block.split("\n")) {
    const trimmed = line.trim().replace(/,$/, "");
    if (!trimmed) continue;
    const idx = trimmed.indexOf(":");
    if (idx === -1) continue;
    result[trimmed.slice(0, idx).trim()] = trimmed.slice(idx + 1).trim();
  }
  return result;
}

export class SparrowClient {
  constructor(private baseUrl: string, private apiKey: string) {}

  private headers(): Record<string, string> {
    return {
      "Content-Type": "application/json",
      "x-api-key": this.apiKey,
    };
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const resp = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify(body),
    });
    if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`);
    return resp.json();
  }

  async hqlEval(query: string): Promise<unknown> {
    return this.post("/__hql_runtime_eval", { query, params: {} });
  }

  async introspect(): Promise<SchemaResponse> {
    const resp = await fetch(`${this.baseUrl}/introspect`, {
      method: "GET",
      headers: this.headers(),
    });
    if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`);
    const data = await resp.json() as { schema: { nodes: SchemaNode[]; edges: SchemaEdge[]; vectors?: unknown[] } };
    return {
      nodes: data.schema?.nodes ?? [],
      edges: data.schema?.edges ?? [],
    };
  }

  async diagnostics(): Promise<DiagnosticsResponse> {
    return this.post("/diagnostics", {});
  }

  async hnswHealth(): Promise<HnswHealthResponse> {
    return this.post("/hnsw_health", {});
  }

  async hnswIntegrity(): Promise<HnswIntegrityResponse> {
    return this.post("/hnsw_integrity", {});
  }

  async vectorSoftDelete(id: string): Promise<void> {
    await this.post("/vector_soft_delete", { id });
  }

  async vectorHardDelete(id: string): Promise<void> {
    await this.post("/vector_hard_delete", { id });
  }

  async purgeSoftDeleted(): Promise<void> {
    await this.post("/purge_soft_deleted", {});
  }

  async rebuildVectorIndex(): Promise<void> {
    await this.post("/rebuild_vector_index", {});
  }
}
