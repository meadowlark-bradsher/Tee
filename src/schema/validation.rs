use crate::proto;

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("node id must not be empty")]
    EmptyNodeId,
    #[error("node type must be specified (got UNSPECIFIED)")]
    UnspecifiedNodeType,
    #[error("node label must not be empty")]
    EmptyNodeLabel,
    #[error("edge source must not be empty")]
    EmptyEdgeSource,
    #[error("edge target must not be empty")]
    EmptyEdgeTarget,
    #[error("self-loops are not permitted (source == target: {0:?})")]
    SelfLoop(String),
    #[error("edge type must be specified (got UNSPECIFIED)")]
    UnspecifiedEdgeType,
    #[error("at least one provenance entry is required")]
    MissingProvenance,
    #[error("provenance source must not be empty")]
    EmptyProvenanceSource,
    #[error("provenance trigger must not be empty")]
    EmptyProvenanceTrigger,
    #[error("incident id must not be empty")]
    EmptyIncidentId,
    #[error("at least one tombstone entry is required")]
    EmptyTombstoneSet,
}

pub fn validate_provenance(prov: &proto::Provenance) -> Result<(), ValidationError> {
    if prov.source.is_empty() {
        return Err(ValidationError::EmptyProvenanceSource);
    }
    if prov.trigger.is_empty() {
        return Err(ValidationError::EmptyProvenanceTrigger);
    }
    Ok(())
}

pub fn validate_node(node: &proto::Node) -> Result<(), ValidationError> {
    if node.id.is_empty() {
        return Err(ValidationError::EmptyNodeId);
    }
    if node.r#type == proto::NodeType::Unspecified as i32 {
        return Err(ValidationError::UnspecifiedNodeType);
    }
    if node.label.is_empty() {
        return Err(ValidationError::EmptyNodeLabel);
    }
    if node.provenance.is_empty() {
        return Err(ValidationError::MissingProvenance);
    }
    for prov in &node.provenance {
        validate_provenance(prov)?;
    }
    Ok(())
}

pub fn validate_edge(edge: &proto::Edge) -> Result<(), ValidationError> {
    if edge.source.is_empty() {
        return Err(ValidationError::EmptyEdgeSource);
    }
    if edge.target.is_empty() {
        return Err(ValidationError::EmptyEdgeTarget);
    }
    if edge.source == edge.target {
        return Err(ValidationError::SelfLoop(edge.source.clone()));
    }
    if edge.r#type == proto::EdgeType::Unspecified as i32 {
        return Err(ValidationError::UnspecifiedEdgeType);
    }
    if edge.provenance.is_empty() {
        return Err(ValidationError::MissingProvenance);
    }
    for prov in &edge.provenance {
        validate_provenance(prov)?;
    }
    Ok(())
}

pub fn validate_hypothesis_delta(delta: &proto::HypothesisDelta) -> Result<(), ValidationError> {
    for node in &delta.nodes {
        validate_node(node)?;
    }
    for edge in &delta.edges {
        validate_edge(edge)?;
    }
    Ok(())
}

pub fn validate_node_tombstone_request(
    req: &proto::NodeTombstoneRequest,
) -> Result<(), ValidationError> {
    if req.incident_id.is_empty() {
        return Err(ValidationError::EmptyIncidentId);
    }
    if req.node_ids.is_empty() {
        return Err(ValidationError::EmptyTombstoneSet);
    }
    match &req.provenance {
        Some(prov) => validate_provenance(prov)?,
        None => return Err(ValidationError::MissingProvenance),
    }
    Ok(())
}

pub fn validate_edge_tombstone_request(
    req: &proto::EdgeTombstoneRequest,
) -> Result<(), ValidationError> {
    if req.incident_id.is_empty() {
        return Err(ValidationError::EmptyIncidentId);
    }
    if req.entries.is_empty() {
        return Err(ValidationError::EmptyTombstoneSet);
    }
    match &req.provenance {
        Some(prov) => validate_provenance(prov)?,
        None => return Err(ValidationError::MissingProvenance),
    }
    for entry in &req.entries {
        if entry.source.is_empty() {
            return Err(ValidationError::EmptyEdgeSource);
        }
        if entry.target.is_empty() {
            return Err(ValidationError::EmptyEdgeTarget);
        }
        if entry.r#type == proto::EdgeType::Unspecified as i32 {
            return Err(ValidationError::UnspecifiedEdgeType);
        }
    }
    Ok(())
}

pub fn validate_incident_id(incident_id: &str) -> Result<(), ValidationError> {
    if incident_id.is_empty() {
        return Err(ValidationError::EmptyIncidentId);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_provenance() -> proto::Provenance {
        proto::Provenance {
            source: "agent-1".into(),
            timestamp: None,
            trigger: "alert-fired".into(),
        }
    }

    fn valid_node() -> proto::Node {
        proto::Node {
            id: "node-1".into(),
            r#type: proto::NodeType::Service as i32,
            label: "api-gateway".into(),
            hypothetical: true,
            provenance: vec![valid_provenance()],
        }
    }

    fn valid_edge() -> proto::Edge {
        proto::Edge {
            source: "node-1".into(),
            target: "node-2".into(),
            r#type: proto::EdgeType::DependsOn as i32,
            provenance: vec![valid_provenance()],
        }
    }

    // --- Node validation ---

    #[test]
    fn valid_node_passes() {
        assert!(validate_node(&valid_node()).is_ok());
    }

    #[test]
    fn empty_node_id_rejected() {
        let mut n = valid_node();
        n.id = "".into();
        assert!(matches!(
            validate_node(&n),
            Err(ValidationError::EmptyNodeId)
        ));
    }

    #[test]
    fn unspecified_node_type_rejected() {
        let mut n = valid_node();
        n.r#type = proto::NodeType::Unspecified as i32;
        assert!(matches!(
            validate_node(&n),
            Err(ValidationError::UnspecifiedNodeType)
        ));
    }

    #[test]
    fn empty_node_label_rejected() {
        let mut n = valid_node();
        n.label = "".into();
        assert!(matches!(
            validate_node(&n),
            Err(ValidationError::EmptyNodeLabel)
        ));
    }

    #[test]
    fn node_missing_provenance_rejected() {
        let mut n = valid_node();
        n.provenance.clear();
        assert!(matches!(
            validate_node(&n),
            Err(ValidationError::MissingProvenance)
        ));
    }

    #[test]
    fn node_empty_provenance_source_rejected() {
        let mut n = valid_node();
        n.provenance[0].source = "".into();
        assert!(matches!(
            validate_node(&n),
            Err(ValidationError::EmptyProvenanceSource)
        ));
    }

    // --- Edge validation ---

    #[test]
    fn valid_edge_passes() {
        assert!(validate_edge(&valid_edge()).is_ok());
    }

    #[test]
    fn empty_edge_source_rejected() {
        let mut e = valid_edge();
        e.source = "".into();
        assert!(matches!(
            validate_edge(&e),
            Err(ValidationError::EmptyEdgeSource)
        ));
    }

    #[test]
    fn self_loop_rejected() {
        let mut e = valid_edge();
        e.target = e.source.clone();
        assert!(matches!(
            validate_edge(&e),
            Err(ValidationError::SelfLoop(_))
        ));
    }

    #[test]
    fn unspecified_edge_type_rejected() {
        let mut e = valid_edge();
        e.r#type = proto::EdgeType::Unspecified as i32;
        assert!(matches!(
            validate_edge(&e),
            Err(ValidationError::UnspecifiedEdgeType)
        ));
    }

    // --- Tombstone validation ---

    #[test]
    fn valid_node_tombstone_passes() {
        let req = proto::NodeTombstoneRequest {
            incident_id: "inc-1".into(),
            node_ids: vec!["n1".into()],
            provenance: Some(valid_provenance()),
        };
        assert!(validate_node_tombstone_request(&req).is_ok());
    }

    #[test]
    fn tombstone_empty_incident_id_rejected() {
        let req = proto::NodeTombstoneRequest {
            incident_id: "".into(),
            node_ids: vec!["n1".into()],
            provenance: Some(valid_provenance()),
        };
        assert!(matches!(
            validate_node_tombstone_request(&req),
            Err(ValidationError::EmptyIncidentId)
        ));
    }

    #[test]
    fn tombstone_empty_ids_rejected() {
        let req = proto::NodeTombstoneRequest {
            incident_id: "inc-1".into(),
            node_ids: vec![],
            provenance: Some(valid_provenance()),
        };
        assert!(matches!(
            validate_node_tombstone_request(&req),
            Err(ValidationError::EmptyTombstoneSet)
        ));
    }

    #[test]
    fn tombstone_missing_provenance_rejected() {
        let req = proto::NodeTombstoneRequest {
            incident_id: "inc-1".into(),
            node_ids: vec!["n1".into()],
            provenance: None,
        };
        assert!(matches!(
            validate_node_tombstone_request(&req),
            Err(ValidationError::MissingProvenance)
        ));
    }
}
