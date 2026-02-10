# Causal Graph Construction: Architecture Overview

## Problem

When diagnosing incidents in complex systems, we need to reason about cause and effect. The space of possible causes is large, and multiple AI agents may be working in parallel to both *generate* hypotheses and *eliminate* them. The challenge is ensuring that:

1. No plausible cause is prematurely excluded
2. Eliminations are durable, consistent, and recoverable
3. The order in which agents work doesn't affect the final result
4. The system is observable at every stage

## Two-Phase Approach

The architecture separates causal reasoning into two distinct phases, each with different guarantees.

### Join Phase: Expanding the Hypothesis Space

Multiple AI agents independently propose causal hypotheses — potential root causes, dependencies, failure mechanisms, and propagation paths. These are accumulated into a shared **causal graph** stored in a graph database.

The key invariant: **hypotheses can only be added, never removed.** This ensures that no agent can accidentally discard another agent's work. The graph only grows.

This phase runs continuously as background knowledge is built up. It is not tied to any specific incident.

### Meet Phase: Narrowing Down for a Specific Incident

When an incident occurs, an **incident ID** is registered. No copy of the hypothesis graph is made — the main graph is shared across all incidents.

Elimination agents then work to rule out hypotheses that don't apply to this incident. Each elimination is recorded as a **tombstone** — a permanent marker, scoped to the incident, that a hypothesis has been ruled out. Tombstones, like hypotheses, can only be added. An eliminated hypothesis stays eliminated.

The live set of remaining hypotheses for a given incident is computed at query time: everything in the main graph minus that incident's tombstones. Multiple incidents share the same underlying graph, each with their own independent tombstone set.

## System Components

```
┌─────────────────────────────────────────────────────────────┐
│                     AI Agent Layer                          │
│                                                             │
│   Hypothesis Agents          Elimination Agents             │
│   (propose causes)           (rule out causes)              │
└──────────┬───────────────────────────┬──────────────────────┘
           │                           │
           │                           ▼
           │                  ┌─────────────────┐
           │                  │      CMBS        │
           │                  │                  │
           │                  │  Observability   │
           │                  │  Guardrails      │
           │                  │  Learning signal │
           │                  └────────┬─────────┘
           │                           │
           ▼                           ▼
    ┌─────────────────────────────────────────┐
    │                  Tee                     │
    │                                         │
    │   Validates and persists all graph       │
    │   mutations with merge guarantees        │
    └──────────────────┬──────────────────────┘
                       │
                       ▼
    ┌─────────────────────────────────────────┐
    │               Neo4j                      │
    │                                         │
    │   Main hypothesis graph (grows only)    │
    │   Per-incident tombstone sets (grow)    │
    └─────────────────────────────────────────┘
```

### Tee

A lightweight service named after the plumbing fitting. All graph mutations flow through Tee, which ensures:

- **Writes are idempotent.** Sending the same hypothesis or tombstone twice has no additional effect. This makes retries safe and simplifies agent design.
- **Writes are order-independent.** Two agents submitting hypotheses in either order produce the same graph. Two agents tombstoning in either order produce the same elimination result. There is no need for coordination between agents.
- **Schema is enforced.** Only permitted node and edge types are accepted. Malformed or out-of-scope submissions are rejected before reaching the database.
- **State is recoverable.** Tee itself holds no state — everything is in Neo4j. If Tee restarts, nothing is lost.

### CMBS

CMBS (Causal Model-Based Supervision) sits between elimination agents and Tee. It operates at progressively deeper levels of capability:

**Level 1 — Observability.** CMBS watches every elimination that passes through it. It detects when agents submit redundant eliminations, fail to recall their own prior work, or deviate from information-theoretically optimal elimination rates. At this level, CMBS is purely passive — every elimination is forwarded to Tee unchanged.

**Level 2 — Guardrails.** CMBS can reject low-quality or redundant elimination requests before they reach Tee. Because skipping an elimination is always conservative (the hypothesis remains live rather than being incorrectly eliminated), this is a safe intervention.

**Level 3 — Learning signal.** CMBS scores each elimination against an information-theoretic baseline. How much did this elimination reduce uncertainty relative to the optimal choice? This signal can be used to train better elimination agents over time.

**Level 4 — Intervention.** Because the final elimination result is the same regardless of the order eliminations are applied, CMBS can rewrite an agent's context history. It can present eliminations in a different order, or show the minimal set of eliminations that would have reached the same result. This influences agent behavior without affecting the mathematical outcome.

If CMBS goes down, it recovers by reading the current state from Tee. No elimination data is lost — Tee and Neo4j are the durable layer.

### Neo4j

The graph database serves as the durable store for all causal graph state. It holds:

- The **main hypothesis graph**, which accumulates hypotheses from all agents across all time
- **Per-incident copies**, each containing the hypothesis snapshot and a growing set of tombstones

Neo4j's built-in `MERGE` operation (create-if-not-exists) aligns naturally with the idempotent write guarantees the system requires.

## Key Design Properties

### Monotonicity

Both phases are monotonic — state only moves in one direction. Hypotheses only accumulate. Tombstones only accumulate. This eliminates an entire class of consistency bugs (lost updates, conflicting writes, race conditions between agents).

### Order Independence

The mathematical properties of the underlying data structures guarantee that applying the same set of operations in any order produces the same result. This means:

- Agents don't need to coordinate with each other
- Retries are always safe
- Partial failures don't corrupt state
- The system is naturally parallelizable

### Separation of Concerns

| Component | Owns | Doesn't own |
|-----------|------|-------------|
| Hypothesis agents | What to propose | Whether it survives |
| Elimination agents | What to rule out | Whether the elimination is accepted |
| CMBS | Quality of eliminations | The durable elimination record |
| Tee | Schema enforcement, persistence | Diagnostic reasoning |
| Neo4j | Durable storage | Application semantics |

### Incident Isolation

Each incident gets its own tombstone set, not its own copy of the graph. Eliminations for incident A have no effect on incident B or on the main hypothesis graph. The shared hypothesis graph continues to grow independently. Storage per incident is proportional to the number of eliminations, not the size of the hypothesis space.

## Lifecycle of an Incident

1. **Background**: Hypothesis agents continuously grow the main causal graph via Tee
2. **Incident trigger**: An incident ID is registered with Tee (O(1), no graph copy)
3. **Investigation**: Elimination agents submit tombstones through CMBS, which observes/filters/scores them before forwarding to Tee. Tombstones are scoped to the incident and reference main graph nodes by ID.
4. **Resolution**: The live view (main graph minus incident tombstones) narrows to the root cause
5. **Post-incident**: CMBS metrics inform agent training; the tombstone set persists for audit

## What This Architecture Does Not Do

- **It does not decide root cause.** The architecture provides the substrate for causal reasoning, not the reasoning itself. Diagnosis is the agents' job.
- **It does not require a specific AI model.** Any agent that can produce hypothesis nodes/edges or tombstone requests can participate.
- **It does not require agents to agree.** Conflicting hypotheses coexist in the Join Phase. The Meet Phase resolves conflicts through elimination, not consensus.
