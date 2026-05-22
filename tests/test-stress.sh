#!/usr/bin/env bash
set -euo pipefail

# ── Config ──────────────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SPARROW_BIN="$REPO_ROOT/target/debug/sparrow"
CONTAINER_BIN="$REPO_ROOT/target/debug/sparrow-container"
PROJECT_DIR="/tmp/sparrow-stress-project"
DATA_DIR="/tmp/sparrow-stress-data"
QUERIES_RS="$REPO_ROOT/crates/sparrow-container/src/queries.rs"
QUERIES_RS_BAK="/tmp/sparrow-stress-queries.rs.bak"
PORT=6969
SERVER_PID=""

# ── Cleanup ──────────────────────────────────────────────────────────────────
cleanup() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        echo ""
        echo "Stopping server (PID $SERVER_PID)..."
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    if [ -f "$QUERIES_RS_BAK" ]; then
        echo "Restoring original queries.rs..."
        cp "$QUERIES_RS_BAK" "$QUERIES_RS"
        rm -f "$QUERIES_RS_BAK"
    fi
    rm -rf "$PROJECT_DIR" "$DATA_DIR"
}
trap cleanup EXIT INT TERM

# ── Check port is free ────────────────────────────────────────────────────────
if lsof -iTCP:"$PORT" -sTCP:LISTEN -P 2>/dev/null | grep -q "$PORT"; then
    echo "ERROR: Port $PORT is already in use. Kill the process first."
    exit 1
fi

# ── Step 1: Create project ────────────────────────────────────────────────────
echo "=== Step 1: Creating stress-test project ==="
rm -rf "$PROJECT_DIR"
mkdir -p "$PROJECT_DIR"

cat > "$PROJECT_DIR/sparrow.toml" << 'EOF'
[project]
name = "stress-test"
queries = "."

[local.dev]
port = 6969
build_mode = "dev"

[cloud]
EOF

cat > "$PROJECT_DIR/schema.hx" << 'EOF'
N::People {
    UNIQUE INDEX person_id: String,
    first_name: String,
    last_name: String,
    age: I32
}

N::Company {
    UNIQUE INDEX name: String
}

N::Jobs {
    UNIQUE INDEX profession: String
}

E::WorksAt UNIQUE {
    From: People,
    To: Company,
    Properties: {}
}

E::HasJob UNIQUE {
    From: People,
    To: Jobs,
    Properties: {}
}

E::OffersJob UNIQUE {
    From: Company,
    To: Jobs,
    Properties: {}
}

V::SkillVec {
    skill_label: String,
    tag: String
}
EOF

cat > "$PROJECT_DIR/queries.hx" << 'EOF'
// ── Write mutations ───────────────────────────────────────────────────────────

QUERY createPerson(person_id: String, first_name: String, last_name: String, age: I32) =>
    person <- AddN<People>({
        person_id: person_id,
        first_name: first_name,
        last_name: last_name,
        age: age
    })
RETURN person

QUERY createCompany(name: String) =>
    company <- AddN<Company>({name: name})
RETURN company

QUERY createJob(profession: String) =>
    job <- AddN<Jobs>({profession: profession})
RETURN job

QUERY ConnectPersonToCompany(person_id: String, company_name: String) =>
    person <- N<People>({person_id: person_id})
    company <- N<Company>({name: company_name})
    edge <- AddE<WorksAt>()::From(person)::To(company)
RETURN edge

QUERY ConnectPersonToJob(person_id: String, profession: String) =>
    person <- N<People>({person_id: person_id})
    job <- N<Jobs>({profession: profession})
    edge <- AddE<HasJob>()::From(person)::To(job)
RETURN edge

QUERY ConnectCompanyToJob(company_name: String, profession: String) =>
    company <- N<Company>({name: company_name})
    job <- N<Jobs>({profession: profession})
    edge <- AddE<OffersJob>()::From(company)::To(job)
RETURN edge

// ── Scan queries ──────────────────────────────────────────────────────────────

QUERY getAllPeople() =>
    people <- N<People>
RETURN people

QUERY getAllCompanies() =>
    companies <- N<Company>
RETURN companies

QUERY getAllJobs() =>
    jobs <- N<Jobs>
RETURN jobs

// ── Point lookups by INDEX field ──────────────────────────────────────────────

QUERY getCompanyByName(name: String) =>
    company <- N<Company>({name: name})
RETURN company

QUERY getJobByProfession(profession: String) =>
    job <- N<Jobs>({profession: profession})
RETURN job

// ── Traversals from People ────────────────────────────────────────────────────

QUERY GetPersonCompany(person_id: String) =>
    person <- N<People>({person_id: person_id})
    company <- person::Out<WorksAt>
RETURN company

QUERY GetPersonJob(person_id: String) =>
    person <- N<People>({person_id: person_id})
    job <- person::Out<HasJob>
RETURN job

QUERY GetPersonFullInfo(person_id: String) =>
    person <- N<People>({person_id: person_id})
    company <- person::Out<WorksAt>
    job <- person::Out<HasJob>
RETURN person, company, job

// ── Traversals from Company ───────────────────────────────────────────────────

QUERY getPeopleAtCompany(company_name: String) =>
    company <- N<Company>({name: company_name})
    people <- company::In<WorksAt>
RETURN people

QUERY getCompanyJobs(company_name: String) =>
    company <- N<Company>({name: company_name})
    jobs <- company::Out<OffersJob>
RETURN jobs

// ── Traversals from Jobs ──────────────────────────────────────────────────────

QUERY getPeopleWithJob(profession: String) =>
    job <- N<Jobs>({profession: profession})
    people <- job::In<HasJob>
RETURN people

QUERY getCompaniesOfferingJob(profession: String) =>
    job <- N<Jobs>({profession: profession})
    companies <- job::In<OffersJob>
RETURN companies

// ── Age range queries ─────────────────────────────────────────────────────────

QUERY getPeopleOlderThan(min_age: I32) =>
    people <- N<People>::WHERE(_::{age}::GT(min_age))
RETURN people

QUERY getPeopleYoungerThan(max_age: I32) =>
    people <- N<People>::WHERE(_::{age}::LT(max_age))
RETURN people

// ── Updates ───────────────────────────────────────────────────────────────────

QUERY updatePersonAge(person_id: String, age: I32) =>
    person <- N<People>({person_id: person_id})::UPDATE({age: age})
RETURN person

QUERY updatePersonName(person_id: String, first_name: String) =>
    person <- N<People>({person_id: person_id})::UPDATE({first_name: first_name})
RETURN person

// ── Delete operations ─────────────────────────────────────────────────────────

QUERY deletePersonById(person_id: String) =>
    DROP N<People>({person_id: person_id})
RETURN NONE

QUERY deletePersonEdges(person_id: String) =>
    DROP N<People>({person_id: person_id})::OutE<WorksAt>
    DROP N<People>({person_id: person_id})::OutE<HasJob>
RETURN NONE

QUERY personExists(person_id: String) =>
    exists <- EXISTS(N<People>({person_id: person_id}))
RETURN exists

// ── Multi-hop traversals ──────────────────────────────────────────────────────

QUERY getPersonCompanyJobs(person_id: String) =>
    person <- N<People>({person_id: person_id})
    company <- person::Out<WorksAt>
    jobs <- company::Out<OffersJob>
RETURN jobs

// ── Aggregation / pagination ──────────────────────────────────────────────────

QUERY countPeople() =>
    count <- N<People>::COUNT
RETURN count

QUERY countCompanyEmployees(company_name: String) =>
    company <- N<Company>({name: company_name})
    count <- company::In<WorksAt>::COUNT
RETURN count

QUERY getFirstPerson() =>
    person <- N<People>::FIRST
RETURN person

QUERY getPeopleFirstTen() =>
    people <- N<People>::RANGE(0, 10)
RETURN people

// ── BM25 text search ─────────────────────────────────────────────────────────

QUERY searchPeopleByName(query: String, limit: I32) =>
    results <- SearchBM25<People>(query, limit)
RETURN results

// ── HNSW vector operations ────────────────────────────────────────────────────

QUERY addSkillVec(vec: [F64], skill_label: String, tag: String) =>
    v <- AddV<SkillVec>(vec, {skill_label: skill_label, tag: tag})
RETURN v::ID

QUERY searchSkillVecs(vec: [F64], k: I64) =>
    results <- SearchV<SkillVec>(vec, k)
RETURN results

QUERY deleteSkillVecsByTag(tag: String) =>
    DROP V<SkillVec>::WHERE(_::{tag}::EQ(tag))
RETURN NONE

QUERY skillVecsWithTag(tag: String) =>
    results <- V<SkillVec>::WHERE(_::{tag}::EQ(tag))
RETURN results
EOF

echo "Project created at $PROJECT_DIR"

# ── Step 2: Compile queries ───────────────────────────────────────────────────
echo ""
echo "=== Step 2: Compiling HQL queries ==="

# Back up current queries.rs
cp "$QUERIES_RS" "$QUERIES_RS_BAK"

"$SPARROW_BIN" compile \
    --path "$PROJECT_DIR" \
    --output "$REPO_ROOT/sparrow-container/src/"

echo "Compiled → sparrow-container/src/queries.rs"

# ── Step 3: Build container ───────────────────────────────────────────────────
echo ""
echo "=== Step 3: Building sparrow-container ==="
cargo build -p sparrow-container 2>&1

echo "Built → $CONTAINER_BIN"

# ── Step 4: Start server ──────────────────────────────────────────────────────
echo ""
echo "=== Step 4: Starting SparrowDB on port $PORT ==="
mkdir -p "$DATA_DIR"

SPARROW_DATA_DIR="$DATA_DIR" SPARROW_PORT="$PORT" "$CONTAINER_BIN" &
SERVER_PID=$!
echo "Server PID: $SERVER_PID"

# Wait for server to be ready (up to 15 seconds)
echo -n "Waiting for server"
for i in $(seq 1 30); do
    if curl -s --max-time 1 "http://localhost:$PORT/getAllPeople" \
            -X POST -H "Content-Type: application/json" -d '{}' > /dev/null 2>&1; then
        echo " ready!"
        break
    fi
    echo -n "."
    sleep 0.5
    if [ "$i" -eq 30 ]; then
        echo ""
        echo "ERROR: Server did not start in time."
        exit 1
    fi
done

# ── Step 5: Run stress test ───────────────────────────────────────────────────
echo ""
echo "=== Step 5: Running stress test ==="
START_TIME=$(date +%s)

"$SPARROW_BIN" stress \
    --endpoint localhost \
    --port "$PORT" \
    --num-people 1000 \
    --num-companies 50 \
    --num-jobs 20 \
    --workers 20 \
    --progress-interval 100

END_TIME=$(date +%s)

# ── Step 6: Restart server and verify persistence ─────────────────────────────
echo ""
echo "=== Step 6: Restarting server to verify persistence ==="

echo "Stopping server (PID $SERVER_PID)..."
kill "$SERVER_PID" 2>/dev/null || true
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=""

# Wait for port to be freed (up to 10 seconds)
echo -n "Waiting for port $PORT to be free"
for i in $(seq 1 20); do
    if ! lsof -iTCP:"$PORT" -sTCP:LISTEN -P 2>/dev/null | grep -q "$PORT"; then
        echo " free!"
        break
    fi
    echo -n "."
    sleep 0.5
    if [ "$i" -eq 20 ]; then
        echo ""
        echo "ERROR: Port $PORT still in use after 10s"
        exit 1
    fi
done

# Restart with the same data directory
echo "Restarting server with same data dir ($DATA_DIR)..."
SPARROW_DATA_DIR="$DATA_DIR" SPARROW_PORT="$PORT" "$CONTAINER_BIN" &
SERVER_PID=$!
echo "New server PID: $SERVER_PID"

# Wait for server to be ready again
echo -n "Waiting for server"
for i in $(seq 1 30); do
    if curl -s --max-time 1 "http://localhost:$PORT/getAllPeople" \
            -X POST -H "Content-Type: application/json" -d '{}' > /dev/null 2>&1; then
        echo " ready!"
        break
    fi
    echo -n "."
    sleep 0.5
    if [ "$i" -eq 30 ]; then
        echo ""
        echo "ERROR: Restarted server did not come up in time."
        exit 1
    fi
done

echo ""
echo "=== Step 7: Running persistence verification ==="
"$SPARROW_BIN" stress \
    --endpoint localhost \
    --port "$PORT" \
    --num-people 1000 \
    --num-companies 50 \
    --num-jobs 20 \
    --verify-only

echo ""
echo "=== Done (wall time: $(($(date +%s) - START_TIME))s) ==="
