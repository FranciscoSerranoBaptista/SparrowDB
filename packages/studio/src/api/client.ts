export type SchemaNode = { name: string; properties: Record<string, string> };
export type SchemaEdge = { name: string; from: string; to: string; properties: Record<string, string> };
export type SchemaResponse = { nodes: SchemaNode[]; edges: SchemaEdge[] };
export type DiagnosticsResponse = { node_count: number; edge_count: number; db_size_bytes: number; uptime_secs: number };
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

  private async get<T>(path: string): Promise<T> {
    const resp = await fetch(`${this.baseUrl}${path}`, {
      method: "GET",
      headers: this.headers(),
    });
    if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`);
    return resp.text() as unknown as T;
  }

  async hqlEval(query: string): Promise<unknown> {
    return this.post("/__hql_runtime_eval", { query, params: {} });
  }

  async introspect(): Promise<SchemaResponse> {
    const raw = await this.get<string>("/introspect");
    return parseSchema(raw);
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
