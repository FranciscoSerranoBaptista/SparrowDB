use serde_json::json;

pub fn docker_compose() -> String {
    r#"services:
  sparrow:
    image: ghcr.io/sparrowdb/sparrowdb:latest
    ports:
      - "6969:6969"
    volumes:
      - sparrow-data:/data
    environment:
      SPARROW_DATA_DIR: /data
      SPARROW_PORT: "6969"
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "sh", "-c", "cat /proc/1/status | grep -q Sleeping || exit 0"]
      interval: 5s
      timeout: 3s
      retries: 12

volumes:
  sparrow-data:
"#
    .to_string()
}

pub fn schema_hx() -> String {
    r#"N::User {
    INDEX name: String,
    INDEX email: String,
}

E::Follows {
    From: User,
    To: User,
}
"#
    .to_string()
}

pub fn queries_hx() -> String {
    r#"QUERY getAllUsers() =>
    users <- N<User>()
    RETURN users

QUERY getUserById(id: ID) =>
    user <- N<User>(id)
    RETURN user

QUERY getFollowers(id: ID) =>
    followers <- N<User>(id)::In<Follows>
    RETURN followers
"#
    .to_string()
}

pub fn seed_json() -> String {
    let v = json!({
        "request_type": "write",
        "query": {
            "queries": [
                {
                    "ForEach": {
                        "param": "data",
                        "body": [
                            {
                                "Query": {
                                    "name": "created",
                                    "steps": [
                                        {
                                            "AddN": {
                                                "label": "User",
                                                "properties": [
                                                    ["name", {"Expr": {"Param": "name"}}],
                                                    ["email", {"Expr": {"Param": "email"}}],
                                                    ["createdAt", {"Expr": "Timestamp"}]
                                                ]
                                            }
                                        }
                                    ],
                                    "condition": null
                                }
                            }
                        ]
                    }
                }
            ],
            "returns": ["created"]
        },
        "parameters": {
            "data": [
                {"name": "Ada Lovelace", "email": "ada@example.com"},
                {"name": "Grace Hopper", "email": "grace@example.com"},
                {"name": "Katherine Johnson", "email": "katherine@example.com"}
            ]
        },
        "parameter_types": {
            "data": {"Array": "Object"}
        }
    });
    serde_json::to_string_pretty(&v).expect("seed_json serialization failed")
}

pub fn read_json() -> String {
    let v = json!({
        "request_type": "read",
        "query": {
            "queries": [
                {
                    "Query": {
                        "name": "users",
                        "steps": [
                            {"NWhere": {"Eq": ["$label", {"String": "User"}]}},
                            {"Limit": 25},
                            {"ValueMap": ["$id", "name", "email", "createdAt"]}
                        ],
                        "condition": null
                    }
                }
            ],
            "returns": ["users"]
        },
        "parameters": {}
    });
    serde_json::to_string_pretty(&v).expect("read_json serialization failed")
}

const DEFAULT_PROJECT_SPEC: &str = r#"You are building a **Personal CRM** as your default MVP because the user did not specify their own intent. Build exactly this — no extra features.

**Entities and edges:**
- `Contact` — properties: `name` (String), `email` (String), `phone` (String, optional), `createdAt` (Timestamp).
- `Company` — properties: `name` (String), `domain` (String, optional), `createdAt` (Timestamp).
- `Interaction` — properties: `kind` (String, one of `"call" | "email" | "note"`), `note` (String), `loggedAt` (Timestamp).
- `Contact -[WORKS_AT]-> Company` with property `since` (I64, year).
- `Contact -[LOGGED]-> Interaction`.

**Queries to write (one JSON file each under `examples/`):**
1. `examples/seed.json` — seed with 3 Companies, 5 Contacts (linked via WORKS_AT), and 6 Interactions (linked via LOGGED).
2. `examples/add_contact.json` — write request, params `name`, `email`, optional `phone`. Returns created contact id.
3. `examples/add_interaction.json` — params `contactId` (I64), `kind`, `note`. Creates Interaction + LOGGED edge.
4. `examples/list_contacts.json` — read, no params, returns up to 50 contacts.
5. `examples/contacts_at_company.json` — read, param `company` (String). Returns contacts at company.
6. `examples/interactions_for_contact.json` — read, param `contactId` (I64). Returns 10 most recent interactions.
7. `examples/search_contacts.json` — read, param `q` (String). Returns up to 25 contacts whose name starts with `q`."#;

const AGENT_PROMPT_TEMPLATE: &str = r#"<role>
You are a SparrowDB expert. SparrowDB is a graph database that exposes a single HTTP endpoint at {base_url}/v1/query. You write JSON query payloads in the SparrowDB v1/query DSL — the same format as HelixDB. Your job is to implement the user's intent as a working set of example JSON query files.
</role>

<environment>
- SparrowDB is running via `docker compose up -d` on port 6969.
- Base URL: {base_url}
- The project directory already contains:
  - `docker-compose.yml` — starts the SparrowDB container.
  - `schema.hx` — HQL schema (node/edge definitions).
  - `queries.hx` — HQL query stubs.
  - `examples/seed.json` — a starter seed payload (3 User nodes).
  - `examples/read.json` — a starter read payload (all Users).
- Place your example query files under `examples/`.
- Test each file with: `curl -s -X POST {base_url}/v1/query -H 'Content-Type: application/json' -d @examples/<file>.json | jq`
</environment>

<user_intent>
{intent}
</user_intent>

<workflow>
1. **Sketch entities** — identify the node labels, edge labels, and properties you need. Update `schema.hx` if new types are required.
2. **Write seed** — create `examples/seed.json` with realistic sample data covering all node/edge types.
3. **Run seed** — execute `curl -s -X POST {base_url}/v1/query -H 'Content-Type: application/json' -d @examples/seed.json | jq` and confirm success.
4. **Write queries** — create one JSON file per query under `examples/`. Cover the read and write operations listed in your intent.
5. **Test each** — run every file through curl and confirm the response matches expectations.
</workflow>

<json_dsl_quickref>
Every request to POST {base_url}/v1/query uses this envelope:

```json
{{
  "request_type": "read" | "write",
  "query": {{
    "queries": [ ...query objects... ],
    "returns": ["name1", "name2"]
  }},
  "parameters": {{ "key": value }},
  "parameter_types": {{ "key": "TypeHint" }}
}}
```

**Sources (start a traversal):**
- `{{"NWhere": {{"Eq": ["$label", {{"String": "Label"}}]}}}}` — all nodes of a label
- `{{"NWhere": {{"Eq": ["field", {{"String": "value"}}]}}}}` — nodes matching a field
- `{{"N:Ids": ["id1", "id2"]}}` — nodes by ID
- `{{"AddN": {{"label": "Label", "properties": [["field", {{"Expr": {{"Param": "paramName"}}}}]]}}}}` — create node

**Traversals:**
- `{{"Out": "EdgeLabel"}}` — follow outgoing edges
- `{{"In": "EdgeLabel"}}` — follow incoming edges
- `{{"OutE": "EdgeLabel"}}` — outgoing edge objects
- `{{"InE": "EdgeLabel"}}` — incoming edge objects

**Filters:**
- `{{"Where": {{"Eq": ["field", {{"String": "val"}}]}}}}` — filter nodes
- `{{"Limit": 25}}` — cap results
- `{{"Skip": 10}}` — offset

**Mutations:**
- `{{"AddN": {{"label": "L", "properties": [["f", {{"Expr": {{"Param": "p"}}}}]]}}}}` — add node
- `{{"AddE": {{"label": "L", "from": "$from_id", "to": "$to_id", "properties": []}}}}` — add edge
- `{{"SetProperty": {{"name": "field", "value": {{"Expr": {{"Param": "p"}}}}}}}}` — update property
- `{{"Drop": {{}}}}` — delete node/edge

**Terminals:**
- `{{"ValueMap": ["$id", "field1", "field2"]}}` — return as map (include `$id` for the node ID)
- `{{"Count": {{}}}}` — return count

**ForEach (bulk writes):**
```json
{{
  "ForEach": {{
    "param": "items",
    "body": [
      {{"Query": {{"name": "result", "steps": [{{"AddN": {{...}}}}], "condition": null}}}}
    ]
  }}
}}
```
Pass `"parameters": {{"items": [...]}}` and `"parameter_types": {{"items": {{"Array": "Object"}}}}`.

**Timestamps:** use `{{"Expr": "Timestamp"}}` as the value for any timestamp property — SparrowDB fills it in server-side.
</json_dsl_quickref>

<antipatterns>
- Do NOT use GraphQL syntax — SparrowDB uses its own JSON DSL.
- Do NOT omit `"condition": null` in Query objects.
- Do NOT forget `"returns"` in the query envelope — list every named query result you want back.
- Do NOT use `std::process::Command` in any async Rust code that interacts with SparrowDB — it blocks the Tokio runtime.
- Do NOT hardcode node IDs; use parameterized queries with `{{"Expr": {{"Param": "paramName"}}}}`.
- Do NOT mix `request_type: "read"` with mutation steps (AddN, AddE, Drop) — use `"write"` for any mutation.
</antipatterns>
"#;

pub fn chef_prompt(intent: &str) -> String {
    let base_url = "http://localhost:6969";
    let resolved_intent = if intent.trim().is_empty() {
        DEFAULT_PROJECT_SPEC.to_string()
    } else {
        intent.to_string()
    };
    AGENT_PROMPT_TEMPLATE
        .replace("{base_url}", base_url)
        .replace("{intent}", &resolved_intent)
}
