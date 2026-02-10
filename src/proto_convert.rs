use std::collections::BTreeSet;

use crate::domain::edge::{EdgeKey, EdgeLattice};
use crate::domain::edge_type::EdgeType;
use crate::domain::node::NodeLattice;
use crate::domain::node_type::NodeType;
use crate::domain::provenance::Provenance;
use crate::proto;

#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("invalid node type value: {0}")]
    InvalidNodeType(i32),
    #[error("invalid edge type value: {0}")]
    InvalidEdgeType(i32),
}

// --- NodeType conversions ---

impl TryFrom<i32> for NodeType {
    type Error = ConversionError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            x if x == proto::NodeType::Service as i32 => Ok(NodeType::Service),
            x if x == proto::NodeType::Dependency as i32 => Ok(NodeType::Dependency),
            x if x == proto::NodeType::Infrastructure as i32 => Ok(NodeType::Infrastructure),
            x if x == proto::NodeType::Mechanism as i32 => Ok(NodeType::Mechanism),
            other => Err(ConversionError::InvalidNodeType(other)),
        }
    }
}

impl From<NodeType> for i32 {
    fn from(value: NodeType) -> Self {
        match value {
            NodeType::Service => proto::NodeType::Service as i32,
            NodeType::Dependency => proto::NodeType::Dependency as i32,
            NodeType::Infrastructure => proto::NodeType::Infrastructure as i32,
            NodeType::Mechanism => proto::NodeType::Mechanism as i32,
        }
    }
}

// --- EdgeType conversions ---

impl TryFrom<i32> for EdgeType {
    type Error = ConversionError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            x if x == proto::EdgeType::DependsOn as i32 => Ok(EdgeType::DependsOn),
            x if x == proto::EdgeType::PropagatesTo as i32 => Ok(EdgeType::PropagatesTo),
            x if x == proto::EdgeType::ManifestsAs as i32 => Ok(EdgeType::ManifestsAs),
            other => Err(ConversionError::InvalidEdgeType(other)),
        }
    }
}

impl From<EdgeType> for i32 {
    fn from(value: EdgeType) -> Self {
        match value {
            EdgeType::DependsOn => proto::EdgeType::DependsOn as i32,
            EdgeType::PropagatesTo => proto::EdgeType::PropagatesTo as i32,
            EdgeType::ManifestsAs => proto::EdgeType::ManifestsAs as i32,
        }
    }
}

// --- Provenance conversions ---

impl From<proto::Provenance> for Provenance {
    fn from(p: proto::Provenance) -> Self {
        let (secs, nanos) = p
            .timestamp
            .map(|t| (t.seconds, t.nanos))
            .unwrap_or((0, 0));
        Provenance::new(p.source, p.trigger).with_timestamp(secs, nanos)
    }
}

impl From<&Provenance> for proto::Provenance {
    fn from(p: &Provenance) -> Self {
        proto::Provenance {
            source: p.source.clone(),
            trigger: p.trigger.clone(),
            timestamp: Some(prost_types::Timestamp {
                seconds: p.timestamp_seconds,
                nanos: p.timestamp_nanos,
            }),
        }
    }
}

// --- Node conversions ---

/// Convert a proto Node to (id, NodeLattice).
/// Assumes validation has already passed.
pub fn proto_node_to_domain(
    node: proto::Node,
) -> Result<(String, NodeLattice), ConversionError> {
    let node_type = NodeType::try_from(node.r#type)?;
    let provenance: BTreeSet<Provenance> = node.provenance.into_iter().map(Into::into).collect();
    let lattice = NodeLattice::new(node_type, node.label, node.hypothetical, provenance);
    Ok((node.id, lattice))
}

/// Convert a (id, NodeLattice) back to proto Node for responses.
pub fn domain_node_to_proto(id: String, lattice: &NodeLattice) -> proto::Node {
    proto::Node {
        id,
        r#type: lattice
            .node_type
            .as_reveal_ref()
            .map(|t| i32::from(*t))
            .unwrap_or(proto::NodeType::Unspecified as i32),
        label: lattice
            .label
            .as_reveal_ref()
            .cloned()
            .unwrap_or_default(),
        hypothetical: *lattice.hypothetical.as_reveal_ref(),
        provenance: lattice
            .provenance
            .as_reveal_ref()
            .iter()
            .map(Into::into)
            .collect(),
    }
}

// --- Edge conversions ---

/// Convert a proto Edge to (EdgeKey, EdgeLattice).
/// Assumes validation has already passed.
pub fn proto_edge_to_domain(
    edge: proto::Edge,
) -> Result<(EdgeKey, EdgeLattice), ConversionError> {
    let edge_type = EdgeType::try_from(edge.r#type)?;
    let key = EdgeKey::new(edge.source, edge.target, edge_type);
    let provenance: BTreeSet<Provenance> = edge.provenance.into_iter().map(Into::into).collect();
    let lattice = EdgeLattice::new(provenance);
    Ok((key, lattice))
}

/// Convert a (EdgeKey, EdgeLattice) back to proto Edge for responses.
pub fn domain_edge_to_proto(key: &EdgeKey, lattice: &EdgeLattice) -> proto::Edge {
    proto::Edge {
        source: key.source.clone(),
        target: key.target.clone(),
        r#type: i32::from(key.edge_type),
        provenance: lattice
            .provenance
            .as_reveal_ref()
            .iter()
            .map(Into::into)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_type_roundtrip() {
        for nt in [
            NodeType::Service,
            NodeType::Dependency,
            NodeType::Infrastructure,
            NodeType::Mechanism,
        ] {
            let i: i32 = nt.into();
            let back = NodeType::try_from(i).unwrap();
            assert_eq!(nt, back);
        }
    }

    #[test]
    fn edge_type_roundtrip() {
        for et in [
            EdgeType::DependsOn,
            EdgeType::PropagatesTo,
            EdgeType::ManifestsAs,
        ] {
            let i: i32 = et.into();
            let back = EdgeType::try_from(i).unwrap();
            assert_eq!(et, back);
        }
    }

    #[test]
    fn invalid_node_type_rejected() {
        assert!(NodeType::try_from(99).is_err());
    }

    #[test]
    fn unspecified_node_type_rejected() {
        assert!(NodeType::try_from(proto::NodeType::Unspecified as i32).is_err());
    }

    #[test]
    fn provenance_roundtrip() {
        let proto_prov = proto::Provenance {
            source: "agent-1".into(),
            trigger: "alert".into(),
            timestamp: Some(prost_types::Timestamp {
                seconds: 1000,
                nanos: 500,
            }),
        };
        let domain: Provenance = proto_prov.into();
        assert_eq!(domain.source, "agent-1");
        assert_eq!(domain.trigger, "alert");
        assert_eq!(domain.timestamp_seconds, 1000);
        assert_eq!(domain.timestamp_nanos, 500);

        let back: proto::Provenance = (&domain).into();
        assert_eq!(back.source, "agent-1");
        assert_eq!(back.trigger, "alert");
        assert_eq!(back.timestamp.unwrap().seconds, 1000);
    }

    #[test]
    fn node_roundtrip() {
        let proto_node = proto::Node {
            id: "n1".into(),
            r#type: proto::NodeType::Service as i32,
            label: "api-gw".into(),
            hypothetical: true,
            provenance: vec![proto::Provenance {
                source: "a".into(),
                trigger: "t".into(),
                timestamp: None,
            }],
        };
        let (id, lattice) = proto_node_to_domain(proto_node).unwrap();
        assert_eq!(id, "n1");

        let back = domain_node_to_proto(id, &lattice);
        assert_eq!(back.id, "n1");
        assert_eq!(back.r#type, proto::NodeType::Service as i32);
        assert_eq!(back.label, "api-gw");
        assert!(back.hypothetical);
        assert_eq!(back.provenance.len(), 1);
    }

    #[test]
    fn edge_roundtrip() {
        let proto_edge = proto::Edge {
            source: "a".into(),
            target: "b".into(),
            r#type: proto::EdgeType::DependsOn as i32,
            provenance: vec![proto::Provenance {
                source: "agent".into(),
                trigger: "scan".into(),
                timestamp: None,
            }],
        };
        let (key, lattice) = proto_edge_to_domain(proto_edge).unwrap();
        assert_eq!(key.source, "a");
        assert_eq!(key.target, "b");
        assert_eq!(key.edge_type, EdgeType::DependsOn);

        let back = domain_edge_to_proto(&key, &lattice);
        assert_eq!(back.source, "a");
        assert_eq!(back.target, "b");
        assert_eq!(back.r#type, proto::EdgeType::DependsOn as i32);
        assert_eq!(back.provenance.len(), 1);
    }
}
