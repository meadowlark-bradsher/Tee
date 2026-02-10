use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::proto::tee_server::Tee;
use crate::proto::{
    CausalGraph, CreateIncidentRequest, CreateIncidentResult, EdgeTombstoneRequest,
    HypothesisDelta, HypothesisMergeResult, IncidentContext, IncidentContextRequest,
    LiveViewRequest, NodeTombstoneRequest, TombstoneMergeResult, TombstoneRequest, TombstoneSet,
};
use crate::schema::validation;
use crate::store::memory::InMemoryStore;
use crate::store::{Store, StoreError};

pub struct TeeService {
    store: Arc<InMemoryStore>,
}

impl TeeService {
    pub fn new(store: Arc<InMemoryStore>) -> Self {
        Self { store }
    }
}

fn store_error_to_status(err: StoreError) -> Status {
    match err {
        StoreError::IncidentNotFound(id) => Status::not_found(format!("incident not found: {id}")),
        StoreError::Backend(msg) => Status::internal(msg),
    }
}

fn validation_error_to_status(err: validation::ValidationError) -> Status {
    Status::invalid_argument(err.to_string())
}

#[tonic::async_trait]
impl Tee for TeeService {
    async fn merge_hypothesis(
        &self,
        request: Request<HypothesisDelta>,
    ) -> Result<Response<HypothesisMergeResult>, Status> {
        let delta = request.into_inner();
        validation::validate_hypothesis_delta(&delta).map_err(validation_error_to_status)?;
        let result = self
            .store
            .merge_hypothesis(delta)
            .await
            .map_err(store_error_to_status)?;
        Ok(Response::new(result))
    }

    async fn create_incident(
        &self,
        request: Request<CreateIncidentRequest>,
    ) -> Result<Response<CreateIncidentResult>, Status> {
        let req = request.into_inner();
        validation::validate_incident_id(&req.incident_id)
            .map_err(validation_error_to_status)?;
        let result = self
            .store
            .create_incident(&req.incident_id)
            .await
            .map_err(store_error_to_status)?;
        Ok(Response::new(result))
    }

    async fn get_incident_context(
        &self,
        request: Request<IncidentContextRequest>,
    ) -> Result<Response<IncidentContext>, Status> {
        let req = request.into_inner();
        validation::validate_incident_id(&req.incident_id)
            .map_err(validation_error_to_status)?;
        let result = self
            .store
            .get_incident_context(&req.incident_id)
            .await
            .map_err(store_error_to_status)?;
        Ok(Response::new(result))
    }

    async fn merge_node_tombstones(
        &self,
        request: Request<NodeTombstoneRequest>,
    ) -> Result<Response<TombstoneMergeResult>, Status> {
        let req = request.into_inner();
        validation::validate_node_tombstone_request(&req)
            .map_err(validation_error_to_status)?;
        let result = self
            .store
            .merge_node_tombstones(req)
            .await
            .map_err(store_error_to_status)?;
        Ok(Response::new(result))
    }

    async fn merge_edge_tombstones(
        &self,
        request: Request<EdgeTombstoneRequest>,
    ) -> Result<Response<TombstoneMergeResult>, Status> {
        let req = request.into_inner();
        validation::validate_edge_tombstone_request(&req)
            .map_err(validation_error_to_status)?;
        let result = self
            .store
            .merge_edge_tombstones(req)
            .await
            .map_err(store_error_to_status)?;
        Ok(Response::new(result))
    }

    async fn get_live_view(
        &self,
        request: Request<LiveViewRequest>,
    ) -> Result<Response<CausalGraph>, Status> {
        let req = request.into_inner();
        validation::validate_incident_id(&req.incident_id)
            .map_err(validation_error_to_status)?;
        let result = self
            .store
            .get_live_view(&req.incident_id)
            .await
            .map_err(store_error_to_status)?;
        Ok(Response::new(result))
    }

    async fn get_tombstones(
        &self,
        request: Request<TombstoneRequest>,
    ) -> Result<Response<TombstoneSet>, Status> {
        let req = request.into_inner();
        validation::validate_incident_id(&req.incident_id)
            .map_err(validation_error_to_status)?;
        let result = self
            .store
            .get_tombstones(&req.incident_id)
            .await
            .map_err(store_error_to_status)?;
        Ok(Response::new(result))
    }

    async fn get_main_graph(
        &self,
        _request: Request<()>,
    ) -> Result<Response<CausalGraph>, Status> {
        let result = self
            .store
            .get_main_graph()
            .await
            .map_err(store_error_to_status)?;
        Ok(Response::new(result))
    }
}
