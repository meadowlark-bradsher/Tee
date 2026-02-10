# Why the Tee / CMBS Architecture Is CRDT-Safe

## Overview

The Tee service implements a **join-semilattice execution model** over causal graph mutations. All persistent state evolves monotonically under merge operations that are associative, commutative, and idempotent (ACI). This ensures that concurrent, duplicated, or reordered writes from multiple agents converge to the same state without coordination.

CMBS operates strictly as a **semantic observer and controller** over this monotone substrate; it does not introduce non-monotone state transitions or rollback. As a result, the combined system inherits the standard convergence and fault-tolerance guarantees of CRDTs.

---

## Lattice Structure

The system maintains two independent classes of lattice state:

### 1. Main Hypothesis Graph (Join Phase)

The shared hypothesis graph is a **grow-only set union lattice**:

* Nodes and edges can be added, never removed
* Identity is defined by explicit keys:

  * Nodes: `id`
  * Edges: `(source, target, type)`
* Merging the same hypothesis twice is a no-op

Formally, the graph state forms a join-semilattice under set union. All writes are monotone.

### 2. Per-Incident Tombstone Sets (Meet Phase Encoding)

For each incident, eliminations are recorded as **grow-only tombstone sets**, also implemented as set-union lattices:

* Node tombstones: eliminated node IDs
* Edge tombstones: eliminated edge identities
* Tombstones can only be added, never removed

The *logical* belief state for an incident is a derived view:

```
Live = MainGraph − Tombstones
```

Although belief contraction is conceptually a meet (intersection), it is implemented operationally via monotone growth of tombstones. This is the standard CRDT technique used in 2P-Sets and OR-Sets.

---

## ACI Properties

All Tee-mediated state transitions satisfy the CRDT merge laws:

### Associativity

Merging updates in grouped or incremental fashion yields the same result:

```
merge(a, merge(b, c)) = merge(merge(a, b), c)
```

### Commutativity

The order of concurrent agent updates does not affect the final state:

```
merge(a, b) = merge(b, a)
```

### Idempotence

Duplicate updates (e.g., retries) are harmless:

```
merge(a, a) = a
```

These properties hold for:

* hypothesis node insertion
* hypothesis edge insertion
* node tombstones
* edge tombstones
* provenance accumulation (deduplicated by `(source, trigger)`)

---

## Field-Level Merge Semantics

To preserve ACI semantics beyond mere node existence, Tee enforces **lattice-typed merge rules for node and edge properties**:

| Field           | Merge Semantics                    | Rationale                            |
| --------------- | ---------------------------------- | ------------------------------------ |
| `id`            | Identity key                       | Defines object identity              |
| `type`, `label` | First-write-wins, conflict = error | Structural fields must be consistent |
| `hypothetical`  | `Max<bool>`                        | Once confirmed, never reverts        |
| `provenance`    | `SetUnion`                         | Append-only attribution              |

By rejecting conflicting structural updates rather than overwriting them, Tee avoids violating commutativity or idempotence at the semantic level.

---

## Incident Isolation Without Copying

Each incident maintains its own tombstone lattice but **shares the main hypothesis graph**. Creating an incident is O(1) and records a **universe anchor** identifying the hypothesis universe the incident reasons over.

This provides:

* isolation between incidents
* deterministic recovery
* no duplication of hypothesis state
* no coordination between incidents

---

## Recovery and Fault Tolerance

Tee is stateless. All durable state resides in Neo4j.

On restart:

* CMBS retrieves the incident context from Tee
* Belief state is reconstructed from:

  * the universe anchor
  * the tombstone sets

Because all state is monotone and idempotent:

* partial writes are safe
* retries are safe
* restarts cannot corrupt state
* no rollback or replay is required

---

## Interaction With CMBS

CMBS:

* observes and evaluates eliminations
* may suppress or reorder elimination *requests*
* never mutates persisted state directly
* never introduces non-monotone transitions

Because elimination is order-independent, CMBS interventions cannot change the final belief state — only the *trajectory*. This preserves CRDT convergence while enabling guardrails, diagnostics, and learning signals.

---

## Summary

The Tee / CMBS architecture is CRDT-safe because:

1. All persisted state is join-semilattice-based and monotone
2. All writes are associative, commutative, and idempotent
3. Deletions are encoded via grow-only tombstones
4. Conflicting non-monotone updates are rejected, not resolved
5. Recovery and retries are safe by construction
6. CMBS operates strictly above the lattice layer

As a result, the system converges deterministically under concurrency, partial failure, and reordering — without coordination.
