# Tee

A Rust service that mediates join-semilattice operations between AI agents and a Neo4j graph database. Named after the plumbing fitting — data flows in and branches to where it needs to go.

## Role in the CMBS Architecture

Tee is the **API layer** for causal graph mutations. It sits between hypothesis-generating agents (Join Phase) or elimination agents (Meet Phase) and the Neo4j backing store, ensuring that all writes satisfy lattice merge semantics: associative, commutative, idempotent. Tee itself is stateless and horizontally scalable — **Neo4j transactions and constraints provide the actual serialization guarantees.**

```
JOIN PHASE                                 MEET PHASE

Hypothesis   ─── delta ──► Tee ──► Neo4j    Elim Agents ──► CMBS ──► Tee ──► Neo4j
Agents                     (main graph)                              (tombstones)
```

CMBS intermediates for elimination agents and calls Tee to persist tombstones. If CMBS goes down, it recovers state from Tee (which reads from Neo4j). Neo4j is the source of truth.

## Architecture

### Data Model

Tee manages two kinds of lattice state in Neo4j, both grow-only:

**Main graph** — a `SetUnion` (grow-only) of hypothesis nodes and edges, written during the Join Phase. Nodes and edges can only be added, never removed. This is the shared hypothesis universe across all incidents.

**Tombstone sets** — per-incident, grow-only sets of eliminated IDs. There are two independent tombstone sets per incident:
- **Node tombstones**: `SetUnion` of eliminated node IDs
- **Edge tombstones**: `SetUnion` of eliminated edge IDs (for when you need to eliminate a causal relationship while keeping both endpoint nodes)

No copy of the main graph is made. Creating an incident is O(1) — it just registers an incident ID.

The **live view** for an incident is computed at query time:
- **Live nodes** = all hypothesis nodes minus node-tombstoned IDs
- **Live edges** = all hypothesis edges minus edge-tombstoned IDs, **also excluding** edges whose source or target node is tombstoned

This means tombstoning a node implicitly removes all its edges from the live view without requiring explicit edge tombstones. Edge tombstones exist only for the case where you want to eliminate a relationship while keeping both nodes alive.

Multiple incidents share the same underlying graph without duplication.

### gRPC API

Tee exposes a gRPC service with the following operations:

```
service Tee {
  // --- Join Phase ---
  // Merge a hypothesis delta into the main graph (idempotent)
  rpc MergeHypothesis(HypothesisDelta) returns (HypothesisMergeResult);

  // --- Incident Lifecycle ---
  // Register a new incident (O(1) — no graph copy, records universe_anchor)
  rpc CreateIncident(CreateIncidentRequest) returns (CreateIncidentResult);

  // Get incident context tuple for CMBS recovery/init
  rpc GetIncidentContext(IncidentContextRequest) returns (IncidentContext);

  // --- Meet Phase ---
  // Add node tombstones for an incident (idempotent, references main graph nodes by ID)
  rpc MergeNodeTombstones(NodeTombstoneRequest) returns (TombstoneMergeResult);

  // Add edge tombstones for an incident (idempotent, references edges by (source, target, type))
  rpc MergeEdgeTombstones(EdgeTombstoneRequest) returns (TombstoneMergeResult);

  // --- Read ---
  // Get the live (non-tombstoned) hypothesis set for an incident
  rpc GetLiveView(LiveViewRequest) returns (CausalGraph);

  // Get the current tombstone set for an incident
  rpc GetTombstones(TombstoneRequest) returns (TombstoneSet);

  // Get the full main graph (no incident scoping)
  rpc GetMainGraph(Empty) returns (CausalGraph);
}

// --- Response Types ---

message HypothesisMergeResult {
  repeated string created_ids = 1;       // newly created nodes/edges (first write)
  repeated string merged_ids = 2;        // already existed (provenance appended)
  repeated MergeConflict conflicts = 3;  // rejected due to type/label conflict
}

message MergeConflict {
  string id = 1;
  string field = 2;          // "type" or "label"
  string existing_value = 3;
  string proposed_value = 4;
}

message TombstoneMergeResult {
  repeated string applied_ids = 1;             // newly tombstoned (first write for this incident)
  repeated string already_tombstoned_ids = 2;  // tombstone already existed (idempotent no-op)
  repeated string unmatched_ids = 3;           // not in main graph (accepted, flagged for observability)
}
```

**Why typed merge results matter.** Tee executes `MERGE ... ON CREATE / ON MATCH` inside a
transaction — it knows exactly which tombstones were new vs already present. Returning this
distinction authoritatively means upstream callers (CMBS `TeeStore`) get a correct
`EliminationResult` in a single round-trip, with no race window between "persist" and
"query what happened." The `applied_ids` / `already_tombstoned_ids` split maps directly to
CMBS's `applied_eliminated` / `ignored_eliminated` contract.

### Key Properties

- **Idempotent writes**: Both `MergeHypothesis` and `MergeTombstones` can be retried safely. Merging the same delta twice is a no-op. Identity is determined by explicit keys (node `id`, edge `(source, target, type)`), not by payload contents. Merge results distinguish new writes from idempotent no-ops so callers never need a follow-up query.
- **Concurrent writes**: Multiple agents can call `MergeHypothesis` or `MergeTombstones` in parallel. Order doesn't matter — the lattice merge is commutative.
- **Transaction-per-write**: Every Tee write executes inside a Neo4j transaction. The read-check-write for conflict detection (e.g., first-write-wins on `type`/`label`) is atomic. Neo4j constraints and transactions are the real serialization layer — Tee is a stateless adapter in front of them.
- **Horizontally scalable**: Multiple Tee instances can run concurrently. They hold no in-memory state. Correctness comes from Neo4j, not from Tee-instance coordination.
- **Schema validation**: Tee validates that incoming nodes and edges conform to the declared causal schema before writing. Invalid mutations are rejected at the API boundary.
- **Tombstones for unknown nodes are accepted**: If a tombstone references a `node_id` not in the main graph, Tee accepts and stores it (the operation is still monotone — a no-op on the live view). The tombstone is flagged as `unmatched` for CMBS observability. This avoids introducing coordination coupling between the Join and Meet phases.

## Data Types

### Node Property Merge Semantics

A node's `id` is its identity. All other fields are **lattice fields** — when two agents
propose the same `id` with different attributes, Tee merges per-field rather than
last-writer-wins. This preserves commutativity and idempotence.

| Field | Merge rule | Rationale |
|---|---|---|
| `id` | Identity key | Two proposals with the same `id` are about the same node |
| `type` | First-write-wins (reject conflict) | A node's type is structural; conflicting types indicate a schema error |
| `label` | First-write-wins (reject conflict) | Labels should be consistent; conflicts are flagged to the caller |
| `hypothetical` | `Max(bool)` — once `false`, stays `false` | A node confirmed by the Meet Phase can't become hypothetical again |
| `provenance` | Append-only set (`SetUnion`) | Every agent's contribution is preserved; no provenance is overwritten |

On conflict (e.g., two agents propose different `type` for the same `id`), Tee returns
an error to the second writer. The first write stands. This is safe because rejecting
a write is conservative — the node already exists with valid data.

### Node

```
message Node {
  string id = 1;
  NodeType type = 2;
  string label = 3;
  bool hypothetical = 4;
  repeated Provenance provenance = 5;  // append-only set, not a single value
}

enum NodeType {
  SERVICE = 0;
  DEPENDENCY = 1;
  INFRASTRUCTURE = 2;
  MECHANISM = 3;
}
```

### Edge

The edge identity key is `(source, target, type)`. Provenance is an attribute, not part of identity.
Same merge rules as nodes: provenance is append-only, type is part of the key.

```
message Edge {
  string source = 1;
  string target = 2;
  EdgeType type = 3;
  repeated Provenance provenance = 4;  // append-only set
}

enum EdgeType {
  DEPENDS_ON = 0;
  PROPAGATES_TO = 1;
  MANIFESTS_AS = 2;
}
```

Because Neo4j relationship constraints are limited compared to node constraints,
edges are stored as nodes with label `HypothesisEdge` and a uniqueness constraint on
`(source, target, type)`. Relationships in Neo4j are used for traversal convenience
but the `HypothesisEdge` node is the source of truth for identity and provenance.

### Provenance

Provenance events are identified by `(source, trigger)` — **not by timestamp**.
This ensures that an agent retrying the same operation with a different timestamp
does not create a duplicate provenance entry. The timestamp records when the event
occurred but is not part of the provenance identity.

```
message Provenance {
  string source = 1;                    // who: agent ID or system component
  google.protobuf.Timestamp timestamp = 2;  // when: informational, not part of identity
  string trigger = 3;                   // why: what triggered this operation
}
```

In the `SetUnion` provenance collection, deduplication is by `(source, trigger)`.
If an agent retries with a different timestamp, the existing entry is kept (first-write-wins
on the timestamp for that `(source, trigger)` pair).

## Implementation Plan

### Phase 1: Project Skeleton

- [ ] Initialize Cargo project with workspace structure
- [ ] Add dependencies: `lattices`, `tonic`, `prost`, `tokio`, `neo4rs`, `serde`
- [ ] Define protobuf schema in `proto/tee.proto`
- [ ] Generate Rust types via `tonic-build`

### Phase 2: Core Lattice Layer

- [ ] Define `CausalGraph` using `MapUnion<BTreeMap<NodeId, NodeLattice>>` where per-node fields are lattice-typed:
  - `type`: `Conflict<NodeType>` (first-write-wins, conflict = schema error)
  - `label`: `Conflict<String>` (first-write-wins, conflict = schema error)
  - `hypothetical`: `Max<bool>` (once false, stays false)
  - `provenance`: `SetUnion<BTreeSet<Provenance>>` (append-only)
- [ ] Implement `Node`, `Edge`, `Provenance` types with `Ord`, `Serialize`, `Deserialize`
- [ ] Implement schema validation (permitted node types, edge types, conflict detection)
- [ ] Unit tests for merge idempotence, commutativity, conflict detection, tombstone application

### Phase 3: Neo4j Adapter

- [ ] Implement `Neo4jStore` trait with operations:
  - `merge_nodes(delta)` — transactional read-check-write: conflict detect then MERGE
  - `merge_edges(delta)` — MERGE on `HypothesisEdge` node + traversal relationship
  - `create_incident(id)` — register incident ID (O(1), no graph copy)
  - `merge_node_tombstones(incident_id, node_ids)` — add node tombstones, flag unmatched
  - `merge_edge_tombstones(incident_id, edge_keys)` — add edge tombstones
  - `get_live_view(incident_id)` — live nodes + live edges (excluding tombstoned endpoints)
  - `get_tombstones(incident_id)` — return node and edge tombstone sets
  - `get_main_graph()` — return the full hypothesis graph
- [ ] Integration tests against a Neo4j test instance

### Phase 4: gRPC Service

- [ ] Implement `Tee` gRPC service using `tonic`
- [ ] Wire gRPC handlers to Neo4j adapter
- [ ] Add request validation and error handling
- [ ] Health check endpoint (`tonic-health`)
- [ ] Integration tests: full round-trip (gRPC → lattice merge → Neo4j → read back)

### Phase 5: CMBS Integration

- [ ] Expose `GetTombstones` and `GetLiveView` for CMBS recovery
- [ ] Verify CMBS can reconstruct state from Tee after restart
- [ ] Load test: concurrent `MergeTombstones` calls from multiple agents

## Dependencies

```toml
[dependencies]
lattices = { version = "0.6", features = ["serde"] }
tonic = "0.14"
prost = "0.13"
prost-types = "0.13"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
neo4rs = "0.8"
serde = { version = "1", features = ["derive"] }

[build-dependencies]
tonic-build = "0.14"
```

## Neo4j Schema

### Constraints

```cypher
// Node identity
CREATE CONSTRAINT hypothesis_id IF NOT EXISTS
FOR (n:Hypothesis) REQUIRE n.id IS UNIQUE;

// Edge identity: (source, target, type) stored as a node for constraint support
CREATE CONSTRAINT edge_identity IF NOT EXISTS
FOR (e:HypothesisEdge) REQUIRE (e.source, e.target, e.type) IS UNIQUE;

// Incident identity
CREATE CONSTRAINT incident_id IF NOT EXISTS
FOR (n:Incident) REQUIRE n.incident_id IS UNIQUE;

// Node tombstone identity: one per (incident, node) pair
CREATE CONSTRAINT node_tombstone_unique IF NOT EXISTS
FOR (t:NodeTombstone) REQUIRE (t.incident_id, t.node_id) IS UNIQUE;

// Edge tombstone identity: one per (incident, source, target, type)
CREATE CONSTRAINT edge_tombstone_unique IF NOT EXISTS
FOR (t:EdgeTombstone) REQUIRE (t.incident_id, t.source, t.target, t.type) IS UNIQUE;
```

### Join Phase writes

All writes execute inside a Neo4j transaction.

```cypher
// --- Node merge (inside transaction) ---
// Step 1: Check for conflict (read)
OPTIONAL MATCH (existing:Hypothesis {id: $id})
RETURN existing.type AS existing_type, existing.label AS existing_label

// Step 2 (Tee application logic):
//   If existing_type is not null AND existing_type <> $type → reject with conflict error
//   If existing_label is not null AND existing_label <> $label → reject with conflict error
//   Otherwise proceed to write:

// Step 3: Write (create or append provenance)
MERGE (n:Hypothesis {id: $id})
ON CREATE SET
  n.type = $type,
  n.label = $label,
  n.hypothetical = true,
  n.provenance_keys = [$prov_key],
  n.provenance_events = [$prov_event]
ON MATCH SET
  n.hypothetical = (n.hypothetical AND $hypothetical),
  n.provenance_keys = CASE
    WHEN NOT $prov_key IN n.provenance_keys
    THEN n.provenance_keys + [$prov_key]
    ELSE n.provenance_keys
  END,
  n.provenance_events = CASE
    WHEN NOT $prov_key IN n.provenance_keys
    THEN n.provenance_events + [$prov_event]
    ELSE n.provenance_events
  END
// prov_key = source + "|" + trigger (dedup identity)
// prov_event = full serialized provenance (source, timestamp, trigger)

// --- Edge merge (inside transaction) ---
// Edges stored as nodes for constraint support, plus a relationship for traversal
MERGE (e:HypothesisEdge {source: $source, target: $target, type: $edge_type})
ON CREATE SET
  e.provenance_keys = [$prov_key],
  e.provenance_events = [$prov_event]
ON MATCH SET
  e.provenance_keys = CASE
    WHEN NOT $prov_key IN e.provenance_keys
    THEN e.provenance_keys + [$prov_key]
    ELSE e.provenance_keys
  END,
  e.provenance_events = CASE
    WHEN NOT $prov_key IN e.provenance_keys
    THEN e.provenance_events + [$prov_event]
    ELSE e.provenance_events
  END

// Also maintain traversal relationship (idempotent)
MATCH (a:Hypothesis {id: $source}), (b:Hypothesis {id: $target})
MERGE (a)-[:CAUSAL {type: $edge_type}]->(b)
```

### Incident creation (O(1) — no graph copy)

```cypher
MERGE (i:Incident {incident_id: $incident_id})
ON CREATE SET i.created_at = timestamp()
```

### Meet Phase writes

Each tombstone MERGE uses `ON CREATE` vs `ON MATCH` to classify the result.
Tee collects these per-ID classifications and returns them in `TombstoneMergeResult`.

```cypher
// Node tombstone (idempotent via MERGE on composite key)
// ON CREATE path → id added to applied_ids
// ON MATCH path  → id added to already_tombstoned_ids
MERGE (t:NodeTombstone {incident_id: $incident_id, node_id: $node_id})
ON CREATE SET t.provenance_key = $prov_key, t.provenance_event = $prov_event,
              t.unmatched = NOT EXISTS { MATCH (:Hypothesis {id: $node_id}) },
              t._created = true
ON MATCH SET  t._created = false
RETURN t._created AS was_created, t.unmatched AS unmatched
// Tee application logic:
//   was_created=true, unmatched=true  → unmatched_ids
//   was_created=true, unmatched=false → applied_ids
//   was_created=false                 → already_tombstoned_ids

// Edge tombstone (same pattern)
MERGE (t:EdgeTombstone {incident_id: $incident_id,
                        source: $source, target: $target, type: $edge_type})
ON CREATE SET t.provenance_key = $prov_key, t.provenance_event = $prov_event,
              t._created = true
ON MATCH SET  t._created = false
RETURN t._created AS was_created
```

### Live view queries

```cypher
// Live nodes: main graph minus node tombstones for this incident
MATCH (n:Hypothesis)
WHERE NOT EXISTS {
  MATCH (t:NodeTombstone {incident_id: $incident_id, node_id: n.id})
}
RETURN n

// Live edges: exclude edge-tombstoned AND edges with tombstoned endpoints
MATCH (e:HypothesisEdge)
WHERE NOT EXISTS {
  MATCH (t:EdgeTombstone {incident_id: $incident_id,
                          source: e.source, target: e.target, type: e.type})
}
AND NOT EXISTS {
  MATCH (t:NodeTombstone {incident_id: $incident_id, node_id: e.source})
}
AND NOT EXISTS {
  MATCH (t:NodeTombstone {incident_id: $incident_id, node_id: e.target})
}
RETURN e

// All tombstones for an incident (for CMBS recovery)
MATCH (t:NodeTombstone {incident_id: $incident_id})
RETURN 'node' AS kind, t.node_id AS id, t.unmatched AS unmatched
UNION ALL
MATCH (t:EdgeTombstone {incident_id: $incident_id})
RETURN 'edge' AS kind, t.source + '|' + t.target + '|' + t.type AS id, false AS unmatched
```
