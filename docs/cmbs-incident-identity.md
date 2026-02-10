# CMBS Incident Identity & Persistence: Design Note

## Summary

CMBS is a **semantic analysis and control layer** for belief trajectories. It does not own persistence, identity, or incident lifecycle. Instead, CMBS operates over **incident contexts supplied by Tee**, which serves as the authoritative registry for belief universes and monotone eliminations.

This separation keeps CMBS stateless, composable, and substrate-agnostic, while preserving all epistemic guarantees (order invariance, irreversibility, recovery).

---

## Design Choice

**Option A: Tee provides incident bookkeeping (selected)**

CMBS does **not** maintain its own backend or database. It relies on Tee to provide the minimal incident context required to reconstruct belief state.

This information is already available in Tee "for free" as part of its join-lattice execution and incident handling.

---

## CMBS Requirements (Minimal)

To resume or analyze an incident, CMBS needs exactly the following tuple:

```
(incident_id, universe_anchor, elimination_set_id)
```

Where:

* **incident_id**
  Stable identifier for the belief process / incident.

* **universe_anchor**
  Identifies the hypothesis universe the incident reasons over
  (e.g. base graph version, snapshot id, or frozen universe identity).

* **elimination_set_id**
  Identifies the grow-only join-lattice of eliminations (tombstones).

Everything else (live hypotheses, belief entropy, survival state, diagnostics) is **derived**.

---

## Responsibilities by Layer

### CMBS (Semantic Layer)

* Accepts `incident_id` as input
* Retrieves incident context from Tee
* Reconstructs belief state as:

  ```
  Live = Universe - Eliminations
  ```
* Provides:

  * observability (belief size, entropy, collapse)
  * guardrails (premature collapse, irreversibility)
  * interventions (trajectory reordering, shock analysis)
  * training signals

CMBS **does not**:

* create incidents
* persist belief state
* manage lifecycle
* talk directly to Neo4j

---

### Tee (Operational / Persistence Layer)

* Owns incident lifecycle
* Owns universe anchoring
* Owns join-semilattice state (hypotheses, eliminations)
* Persists incident metadata and tombstones
* Exposes a minimal incident-context API to CMBS

---

### Storage (Neo4j or equivalent)

* Stores graph state, tombstones, and incident metadata
* Has no knowledge of CMBS semantics
* Is accessed only via Tee

---

## Recovery Semantics

On restart or handoff:

1. CMBS requests incident context from Tee using `incident_id`
2. CMBS reconstructs belief state from:

   * universe at `universe_anchor`
   * elimination join-lattice
3. CMBS resumes analysis with no loss of epistemic guarantees

This avoids ambiguity about:

* newly added hypotheses
* belief resurrection
* irreversibility violations

No coordination, rollback, or replay is required.

---

## Why This Works

* **Order invariance** is preserved (join-lattice execution)
* **Irreversibility** is explicit (tombstones only grow)
* **Recovery is exact** (universe anchor removes ambiguity)
* **CMBS remains a lens**, not a datastore
* **No duplication of responsibility** between CMBS and Tee

This mirrors proven CRDT and CALM-style separations:

* execution vs interpretation
* storage vs semantics

---

## Explicit Non-Goals

* CMBS does not provide a database
* CMBS does not manage incident identity
* CMBS does not coordinate updates
* CMBS does not discover new hypotheses

Those concerns belong upstream.

---

## One-Line Contract

> **CMBS operates over incident contexts supplied by Tee; Tee is the authoritative registry for belief universes and monotone eliminations.**
