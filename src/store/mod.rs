pub mod memory;

use crate::proto;

/// Errors from the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("incident not found: {0}")]
    IncidentNotFound(String),
    #[error("storage backend error: {0}")]
    Backend(String),
}

/// The storage trait that Tee's gRPC handlers delegate to.
///
/// Each method corresponds to a gRPC RPC. Implementations include:
/// - `Neo4jStore` (Phase 3) — the production backend
/// - `InMemoryStore` (future) — for testing without Neo4j
#[allow(async_fn_in_trait)]
pub trait Store: Send + Sync {
    async fn merge_hypothesis(
        &self,
        delta: proto::HypothesisDelta,
    ) -> Result<proto::HypothesisMergeResult, StoreError>;

    async fn create_incident(
        &self,
        incident_id: &str,
    ) -> Result<proto::CreateIncidentResult, StoreError>;

    async fn get_incident_context(
        &self,
        incident_id: &str,
    ) -> Result<proto::IncidentContext, StoreError>;

    async fn merge_node_tombstones(
        &self,
        request: proto::NodeTombstoneRequest,
    ) -> Result<proto::TombstoneMergeResult, StoreError>;

    async fn merge_edge_tombstones(
        &self,
        request: proto::EdgeTombstoneRequest,
    ) -> Result<proto::TombstoneMergeResult, StoreError>;

    async fn get_live_view(
        &self,
        incident_id: &str,
    ) -> Result<proto::CausalGraph, StoreError>;

    async fn get_tombstones(
        &self,
        incident_id: &str,
    ) -> Result<proto::TombstoneSet, StoreError>;

    async fn get_main_graph(&self) -> Result<proto::CausalGraph, StoreError>;
}
