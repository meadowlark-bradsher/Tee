use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use lattices::Merge;
use tokio::sync::RwLock;

use crate::domain::edge::{EdgeKey, EdgeLattice};
use crate::domain::edge_type::EdgeType;
use crate::domain::node::NodeLattice;
use crate::proto;
use crate::proto_convert::{
    domain_edge_to_proto, domain_node_to_proto, proto_edge_to_domain, proto_node_to_domain,
};

use super::{Store, StoreError};

/// Per-incident state tracking tombstones and creation time.
#[derive(Debug)]
struct IncidentState {
    created_at: (i64, i32),
    node_tombstones: BTreeSet<String>,
    edge_tombstones: BTreeSet<EdgeKey>,
}

/// Internal mutable state behind the RwLock.
#[derive(Debug, Default)]
struct InnerState {
    nodes: BTreeMap<String, NodeLattice>,
    edges: BTreeMap<EdgeKey, EdgeLattice>,
    incidents: BTreeMap<String, IncidentState>,
}

/// In-memory implementation of the [`Store`] trait.
///
/// All state is held behind a [`RwLock`] for concurrent access.
/// Uses the lattice-backed domain types directly — no external dependencies.
#[derive(Debug, Clone)]
pub struct InMemoryStore {
    state: Arc<RwLock<InnerState>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(InnerState::default())),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Store for InMemoryStore {
    async fn merge_hypothesis(
        &self,
        delta: proto::HypothesisDelta,
    ) -> Result<proto::HypothesisMergeResult, StoreError> {
        let mut state = self.state.write().await;
        let mut created_ids = Vec::new();
        let mut merged_ids = Vec::new();
        let mut conflicts = Vec::new();

        // Process nodes
        for proto_node in delta.nodes {
            let node_id = proto_node.id.clone();
            let (id, lattice) = proto_node_to_domain(proto_node)
                .map_err(|e| StoreError::Backend(e.to_string()))?;

            match state.nodes.get_mut(&id) {
                Some(existing) => {
                    // Clone to test merge without polluting state on conflict
                    let mut candidate = existing.clone();
                    candidate.merge(lattice);
                    if candidate.has_conflict() {
                        // Report the conflict — don't persist
                        let field = candidate
                            .conflict_field()
                            .unwrap_or("unknown")
                            .to_string();
                        let existing_value = match field.as_str() {
                            "type" => existing
                                .node_type
                                .as_reveal_ref()
                                .map(|t| t.to_string())
                                .unwrap_or_default(),
                            "label" => existing
                                .label
                                .as_reveal_ref()
                                .cloned()
                                .unwrap_or_default(),
                            _ => String::new(),
                        };
                        conflicts.push(proto::MergeConflict {
                            id: node_id,
                            field,
                            existing_value,
                            proposed_value: String::new(), // delta already consumed
                        });
                    } else {
                        *existing = candidate;
                        merged_ids.push(node_id);
                    }
                }
                None => {
                    state.nodes.insert(id, lattice);
                    created_ids.push(node_id);
                }
            }
        }

        // Process edges — edges have no conflict fields (only provenance grows)
        for proto_edge in delta.edges {
            let edge_id = format!("{}->{}:{}", proto_edge.source, proto_edge.target, proto_edge.r#type);
            let (key, lattice) = proto_edge_to_domain(proto_edge)
                .map_err(|e| StoreError::Backend(e.to_string()))?;

            match state.edges.get_mut(&key) {
                Some(existing) => {
                    existing.merge(lattice);
                    merged_ids.push(edge_id);
                }
                None => {
                    state.edges.insert(key, lattice);
                    created_ids.push(edge_id);
                }
            }
        }

        Ok(proto::HypothesisMergeResult {
            created_ids,
            merged_ids,
            conflicts,
        })
    }

    async fn create_incident(
        &self,
        incident_id: &str,
    ) -> Result<proto::CreateIncidentResult, StoreError> {
        let mut state = self.state.write().await;
        let created = if state.incidents.contains_key(incident_id) {
            false
        } else {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            state.incidents.insert(
                incident_id.to_string(),
                IncidentState {
                    created_at: (now.as_secs() as i64, now.subsec_nanos() as i32),
                    node_tombstones: BTreeSet::new(),
                    edge_tombstones: BTreeSet::new(),
                },
            );
            true
        };

        Ok(proto::CreateIncidentResult {
            incident_id: incident_id.to_string(),
            created,
        })
    }

    async fn get_incident_context(
        &self,
        incident_id: &str,
    ) -> Result<proto::IncidentContext, StoreError> {
        let state = self.state.read().await;
        let incident = state
            .incidents
            .get(incident_id)
            .ok_or_else(|| StoreError::IncidentNotFound(incident_id.to_string()))?;

        let tombstones = proto::TombstoneSet {
            node_ids: incident.node_tombstones.iter().cloned().collect(),
            edge_entries: incident
                .edge_tombstones
                .iter()
                .map(|k| proto::EdgeTombstoneEntry {
                    source: k.source.clone(),
                    target: k.target.clone(),
                    r#type: i32::from(k.edge_type),
                })
                .collect(),
        };

        Ok(proto::IncidentContext {
            incident_id: incident_id.to_string(),
            created_at: Some(prost_types::Timestamp {
                seconds: incident.created_at.0,
                nanos: incident.created_at.1,
            }),
            tombstones: Some(tombstones),
        })
    }

    async fn merge_node_tombstones(
        &self,
        request: proto::NodeTombstoneRequest,
    ) -> Result<proto::TombstoneMergeResult, StoreError> {
        let mut state = self.state.write().await;
        let InnerState {
            ref nodes,
            ref mut incidents,
            ..
        } = *state;
        let incident = incidents
            .get_mut(&request.incident_id)
            .ok_or_else(|| StoreError::IncidentNotFound(request.incident_id.clone()))?;

        let mut applied_ids = Vec::new();
        let mut already_tombstoned_ids = Vec::new();
        let mut unmatched_ids = Vec::new();

        for node_id in request.node_ids {
            if incident.node_tombstones.contains(&node_id) {
                already_tombstoned_ids.push(node_id);
            } else {
                incident.node_tombstones.insert(node_id.clone());
                if nodes.contains_key(&node_id) {
                    applied_ids.push(node_id);
                } else {
                    unmatched_ids.push(node_id);
                }
            }
        }

        Ok(proto::TombstoneMergeResult {
            applied_ids,
            already_tombstoned_ids,
            unmatched_ids,
        })
    }

    async fn merge_edge_tombstones(
        &self,
        request: proto::EdgeTombstoneRequest,
    ) -> Result<proto::TombstoneMergeResult, StoreError> {
        let mut state = self.state.write().await;
        let InnerState {
            ref edges,
            ref mut incidents,
            ..
        } = *state;
        let incident = incidents
            .get_mut(&request.incident_id)
            .ok_or_else(|| StoreError::IncidentNotFound(request.incident_id.clone()))?;

        let mut applied_ids = Vec::new();
        let mut already_tombstoned_ids = Vec::new();
        let mut unmatched_ids = Vec::new();

        for entry in request.entries {
            let edge_type = EdgeType::try_from(entry.r#type)
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            let key = EdgeKey::new(&entry.source, &entry.target, edge_type);
            let edge_id = format!("{}->{}:{}", entry.source, entry.target, entry.r#type);

            if incident.edge_tombstones.contains(&key) {
                already_tombstoned_ids.push(edge_id);
            } else {
                incident.edge_tombstones.insert(key.clone());
                if edges.contains_key(&key) {
                    applied_ids.push(edge_id);
                } else {
                    unmatched_ids.push(edge_id);
                }
            }
        }

        Ok(proto::TombstoneMergeResult {
            applied_ids,
            already_tombstoned_ids,
            unmatched_ids,
        })
    }

    async fn get_live_view(
        &self,
        incident_id: &str,
    ) -> Result<proto::CausalGraph, StoreError> {
        let state = self.state.read().await;
        let incident = state
            .incidents
            .get(incident_id)
            .ok_or_else(|| StoreError::IncidentNotFound(incident_id.to_string()))?;

        let nodes: Vec<proto::Node> = state
            .nodes
            .iter()
            .filter(|(id, _)| !incident.node_tombstones.contains(*id))
            .map(|(id, lattice)| domain_node_to_proto(id.clone(), lattice))
            .collect();

        let edges: Vec<proto::Edge> = state
            .edges
            .iter()
            .filter(|(key, _)| {
                !incident.edge_tombstones.contains(*key)
                    && !incident.node_tombstones.contains(&key.source)
                    && !incident.node_tombstones.contains(&key.target)
            })
            .map(|(key, lattice)| domain_edge_to_proto(key, lattice))
            .collect();

        Ok(proto::CausalGraph { nodes, edges })
    }

    async fn get_tombstones(
        &self,
        incident_id: &str,
    ) -> Result<proto::TombstoneSet, StoreError> {
        let state = self.state.read().await;
        let incident = state
            .incidents
            .get(incident_id)
            .ok_or_else(|| StoreError::IncidentNotFound(incident_id.to_string()))?;

        Ok(proto::TombstoneSet {
            node_ids: incident.node_tombstones.iter().cloned().collect(),
            edge_entries: incident
                .edge_tombstones
                .iter()
                .map(|k| proto::EdgeTombstoneEntry {
                    source: k.source.clone(),
                    target: k.target.clone(),
                    r#type: i32::from(k.edge_type),
                })
                .collect(),
        })
    }

    async fn get_main_graph(&self) -> Result<proto::CausalGraph, StoreError> {
        let state = self.state.read().await;

        let nodes: Vec<proto::Node> = state
            .nodes
            .iter()
            .map(|(id, lattice)| domain_node_to_proto(id.clone(), lattice))
            .collect();

        let edges: Vec<proto::Edge> = state
            .edges
            .iter()
            .map(|(key, lattice)| domain_edge_to_proto(key, lattice))
            .collect();

        Ok(proto::CausalGraph { nodes, edges })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str, node_type: i32, label: &str) -> proto::Node {
        proto::Node {
            id: id.into(),
            r#type: node_type,
            label: label.into(),
            hypothetical: true,
            provenance: vec![proto::Provenance {
                source: "agent-1".into(),
                trigger: "alert".into(),
                timestamp: None,
            }],
        }
    }

    fn make_edge(source: &str, target: &str, edge_type: i32) -> proto::Edge {
        proto::Edge {
            source: source.into(),
            target: target.into(),
            r#type: edge_type,
            provenance: vec![proto::Provenance {
                source: "agent-1".into(),
                trigger: "alert".into(),
                timestamp: None,
            }],
        }
    }

    fn make_delta(nodes: Vec<proto::Node>, edges: Vec<proto::Edge>) -> proto::HypothesisDelta {
        proto::HypothesisDelta { nodes, edges }
    }

    // --- merge_hypothesis ---

    #[tokio::test]
    async fn merge_creates_new_nodes() {
        let store = InMemoryStore::new();
        let delta = make_delta(
            vec![make_node("n1", proto::NodeType::Service as i32, "svc")],
            vec![],
        );
        let result = store.merge_hypothesis(delta).await.unwrap();
        assert_eq!(result.created_ids, vec!["n1"]);
        assert!(result.merged_ids.is_empty());
        assert!(result.conflicts.is_empty());
    }

    #[tokio::test]
    async fn merge_idempotent_same_node() {
        let store = InMemoryStore::new();
        let delta = make_delta(
            vec![make_node("n1", proto::NodeType::Service as i32, "svc")],
            vec![],
        );
        store.merge_hypothesis(delta.clone()).await.unwrap();
        let result = store.merge_hypothesis(delta).await.unwrap();
        assert!(result.created_ids.is_empty());
        assert_eq!(result.merged_ids, vec!["n1"]);
        assert!(result.conflicts.is_empty());
    }

    #[tokio::test]
    async fn merge_detects_type_conflict() {
        let store = InMemoryStore::new();
        let delta1 = make_delta(
            vec![make_node("n1", proto::NodeType::Service as i32, "svc")],
            vec![],
        );
        store.merge_hypothesis(delta1).await.unwrap();

        let delta2 = make_delta(
            vec![make_node(
                "n1",
                proto::NodeType::Infrastructure as i32,
                "svc",
            )],
            vec![],
        );
        let result = store.merge_hypothesis(delta2).await.unwrap();
        assert!(result.created_ids.is_empty());
        assert!(result.merged_ids.is_empty());
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].id, "n1");
        assert_eq!(result.conflicts[0].field, "type");
    }

    #[tokio::test]
    async fn merge_conflict_does_not_persist() {
        let store = InMemoryStore::new();
        let delta1 = make_delta(
            vec![make_node("n1", proto::NodeType::Service as i32, "svc")],
            vec![],
        );
        store.merge_hypothesis(delta1).await.unwrap();

        // Attempt conflicting merge
        let delta2 = make_delta(
            vec![make_node(
                "n1",
                proto::NodeType::Infrastructure as i32,
                "svc",
            )],
            vec![],
        );
        store.merge_hypothesis(delta2).await.unwrap();

        // Original should be intact — re-merge same type should work
        let delta3 = make_delta(
            vec![make_node("n1", proto::NodeType::Service as i32, "svc")],
            vec![],
        );
        let result = store.merge_hypothesis(delta3).await.unwrap();
        assert!(result.conflicts.is_empty());
        assert_eq!(result.merged_ids, vec!["n1"]);
    }

    #[tokio::test]
    async fn merge_creates_and_merges_edges() {
        let store = InMemoryStore::new();
        let delta1 = make_delta(
            vec![],
            vec![make_edge("a", "b", proto::EdgeType::DependsOn as i32)],
        );
        let result1 = store.merge_hypothesis(delta1).await.unwrap();
        assert_eq!(result1.created_ids.len(), 1);

        let delta2 = make_delta(
            vec![],
            vec![make_edge("a", "b", proto::EdgeType::DependsOn as i32)],
        );
        let result2 = store.merge_hypothesis(delta2).await.unwrap();
        assert_eq!(result2.merged_ids.len(), 1);
    }

    // --- create_incident ---

    #[tokio::test]
    async fn create_incident_new() {
        let store = InMemoryStore::new();
        let result = store.create_incident("inc-1").await.unwrap();
        assert!(result.created);
        assert_eq!(result.incident_id, "inc-1");
    }

    #[tokio::test]
    async fn create_incident_idempotent() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();
        let result = store.create_incident("inc-1").await.unwrap();
        assert!(!result.created);
    }

    // --- get_incident_context ---

    #[tokio::test]
    async fn get_incident_context_not_found() {
        let store = InMemoryStore::new();
        let result = store.get_incident_context("nope").await;
        assert!(matches!(result, Err(StoreError::IncidentNotFound(_))));
    }

    #[tokio::test]
    async fn get_incident_context_returns_tombstones() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();

        // Add a node to the main graph first
        let delta = make_delta(
            vec![make_node("n1", proto::NodeType::Service as i32, "svc")],
            vec![],
        );
        store.merge_hypothesis(delta).await.unwrap();

        // Tombstone it
        store
            .merge_node_tombstones(proto::NodeTombstoneRequest {
                incident_id: "inc-1".into(),
                node_ids: vec!["n1".into()],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();

        let ctx = store.get_incident_context("inc-1").await.unwrap();
        assert_eq!(ctx.incident_id, "inc-1");
        assert!(ctx.created_at.is_some());
        let tombstones = ctx.tombstones.unwrap();
        assert_eq!(tombstones.node_ids, vec!["n1"]);
    }

    // --- merge_node_tombstones ---

    #[tokio::test]
    async fn tombstone_applied_for_existing_node() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();

        let delta = make_delta(
            vec![make_node("n1", proto::NodeType::Service as i32, "svc")],
            vec![],
        );
        store.merge_hypothesis(delta).await.unwrap();

        let result = store
            .merge_node_tombstones(proto::NodeTombstoneRequest {
                incident_id: "inc-1".into(),
                node_ids: vec!["n1".into()],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();
        assert_eq!(result.applied_ids, vec!["n1"]);
        assert!(result.already_tombstoned_ids.is_empty());
        assert!(result.unmatched_ids.is_empty());
    }

    #[tokio::test]
    async fn tombstone_unmatched_for_missing_node() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();

        let result = store
            .merge_node_tombstones(proto::NodeTombstoneRequest {
                incident_id: "inc-1".into(),
                node_ids: vec!["ghost".into()],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();
        assert!(result.applied_ids.is_empty());
        assert_eq!(result.unmatched_ids, vec!["ghost"]);
    }

    #[tokio::test]
    async fn tombstone_idempotent() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();

        let delta = make_delta(
            vec![make_node("n1", proto::NodeType::Service as i32, "svc")],
            vec![],
        );
        store.merge_hypothesis(delta).await.unwrap();

        let req = proto::NodeTombstoneRequest {
            incident_id: "inc-1".into(),
            node_ids: vec!["n1".into()],
            provenance: Some(proto::Provenance {
                source: "agent".into(),
                trigger: "elim".into(),
                timestamp: None,
            }),
        };
        store.merge_node_tombstones(req.clone()).await.unwrap();
        let result = store.merge_node_tombstones(req).await.unwrap();
        assert!(result.applied_ids.is_empty());
        assert_eq!(result.already_tombstoned_ids, vec!["n1"]);
    }

    // --- merge_edge_tombstones ---

    #[tokio::test]
    async fn edge_tombstone_applied() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();

        let delta = make_delta(
            vec![],
            vec![make_edge("a", "b", proto::EdgeType::DependsOn as i32)],
        );
        store.merge_hypothesis(delta).await.unwrap();

        let result = store
            .merge_edge_tombstones(proto::EdgeTombstoneRequest {
                incident_id: "inc-1".into(),
                entries: vec![proto::EdgeTombstoneEntry {
                    source: "a".into(),
                    target: "b".into(),
                    r#type: proto::EdgeType::DependsOn as i32,
                }],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();
        assert_eq!(result.applied_ids.len(), 1);
    }

    // --- get_live_view ---

    #[tokio::test]
    async fn live_view_filters_tombstoned_nodes() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();

        let delta = make_delta(
            vec![
                make_node("n1", proto::NodeType::Service as i32, "svc1"),
                make_node("n2", proto::NodeType::Service as i32, "svc2"),
            ],
            vec![],
        );
        store.merge_hypothesis(delta).await.unwrap();

        store
            .merge_node_tombstones(proto::NodeTombstoneRequest {
                incident_id: "inc-1".into(),
                node_ids: vec!["n1".into()],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();

        let view = store.get_live_view("inc-1").await.unwrap();
        assert_eq!(view.nodes.len(), 1);
        assert_eq!(view.nodes[0].id, "n2");
    }

    #[tokio::test]
    async fn live_view_filters_edges_of_tombstoned_nodes() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();

        let delta = make_delta(
            vec![
                make_node("n1", proto::NodeType::Service as i32, "svc1"),
                make_node("n2", proto::NodeType::Service as i32, "svc2"),
                make_node("n3", proto::NodeType::Service as i32, "svc3"),
            ],
            vec![
                make_edge("n1", "n2", proto::EdgeType::DependsOn as i32),
                make_edge("n2", "n3", proto::EdgeType::DependsOn as i32),
            ],
        );
        store.merge_hypothesis(delta).await.unwrap();

        // Tombstone n1 — edge n1->n2 should also disappear
        store
            .merge_node_tombstones(proto::NodeTombstoneRequest {
                incident_id: "inc-1".into(),
                node_ids: vec!["n1".into()],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();

        let view = store.get_live_view("inc-1").await.unwrap();
        assert_eq!(view.nodes.len(), 2);
        assert_eq!(view.edges.len(), 1);
        assert_eq!(view.edges[0].source, "n2");
        assert_eq!(view.edges[0].target, "n3");
    }

    #[tokio::test]
    async fn live_view_filters_tombstoned_edges() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();

        let delta = make_delta(
            vec![
                make_node("n1", proto::NodeType::Service as i32, "svc1"),
                make_node("n2", proto::NodeType::Service as i32, "svc2"),
            ],
            vec![make_edge("n1", "n2", proto::EdgeType::DependsOn as i32)],
        );
        store.merge_hypothesis(delta).await.unwrap();

        // Tombstone just the edge
        store
            .merge_edge_tombstones(proto::EdgeTombstoneRequest {
                incident_id: "inc-1".into(),
                entries: vec![proto::EdgeTombstoneEntry {
                    source: "n1".into(),
                    target: "n2".into(),
                    r#type: proto::EdgeType::DependsOn as i32,
                }],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();

        let view = store.get_live_view("inc-1").await.unwrap();
        assert_eq!(view.nodes.len(), 2);
        assert_eq!(view.edges.len(), 0);
    }

    // --- incident isolation ---

    #[tokio::test]
    async fn tombstones_isolated_between_incidents() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();
        store.create_incident("inc-2").await.unwrap();

        let delta = make_delta(
            vec![
                make_node("n1", proto::NodeType::Service as i32, "svc1"),
                make_node("n2", proto::NodeType::Service as i32, "svc2"),
            ],
            vec![],
        );
        store.merge_hypothesis(delta).await.unwrap();

        // Tombstone n1 in inc-1 only
        store
            .merge_node_tombstones(proto::NodeTombstoneRequest {
                incident_id: "inc-1".into(),
                node_ids: vec!["n1".into()],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();

        let view1 = store.get_live_view("inc-1").await.unwrap();
        let view2 = store.get_live_view("inc-2").await.unwrap();
        assert_eq!(view1.nodes.len(), 1); // n1 tombstoned
        assert_eq!(view2.nodes.len(), 2); // both visible
    }

    // --- get_main_graph ---

    #[tokio::test]
    async fn main_graph_includes_all() {
        let store = InMemoryStore::new();
        let delta = make_delta(
            vec![
                make_node("n1", proto::NodeType::Service as i32, "svc1"),
                make_node("n2", proto::NodeType::Service as i32, "svc2"),
            ],
            vec![make_edge("n1", "n2", proto::EdgeType::DependsOn as i32)],
        );
        store.merge_hypothesis(delta).await.unwrap();

        let graph = store.get_main_graph().await.unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
    }

    // --- get_tombstones ---

    #[tokio::test]
    async fn get_tombstones_returns_sets() {
        let store = InMemoryStore::new();
        store.create_incident("inc-1").await.unwrap();

        let delta = make_delta(
            vec![make_node("n1", proto::NodeType::Service as i32, "svc")],
            vec![make_edge("n1", "n2", proto::EdgeType::DependsOn as i32)],
        );
        store.merge_hypothesis(delta).await.unwrap();

        store
            .merge_node_tombstones(proto::NodeTombstoneRequest {
                incident_id: "inc-1".into(),
                node_ids: vec!["n1".into()],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();

        store
            .merge_edge_tombstones(proto::EdgeTombstoneRequest {
                incident_id: "inc-1".into(),
                entries: vec![proto::EdgeTombstoneEntry {
                    source: "n1".into(),
                    target: "n2".into(),
                    r#type: proto::EdgeType::DependsOn as i32,
                }],
                provenance: Some(proto::Provenance {
                    source: "agent".into(),
                    trigger: "elim".into(),
                    timestamp: None,
                }),
            })
            .await
            .unwrap();

        let tombstones = store.get_tombstones("inc-1").await.unwrap();
        assert_eq!(tombstones.node_ids, vec!["n1"]);
        assert_eq!(tombstones.edge_entries.len(), 1);
    }
}
