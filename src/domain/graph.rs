use lattices::map_union::MapUnionBTreeMap;

use super::edge::{EdgeKey, EdgeLattice};
use super::node::NodeLattice;

/// Full hypothesis node map. Used for composite merge testing.
/// In production, Tee merges nodes individually against Neo4j-fetched state.
pub type NodeMap = MapUnionBTreeMap<String, NodeLattice>;

/// Full hypothesis edge map. Same usage pattern as NodeMap.
pub type EdgeMap = MapUnionBTreeMap<EdgeKey, EdgeLattice>;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use lattices::Merge;

    use super::*;
    use crate::domain::edge_type::EdgeType;
    use crate::domain::node_type::NodeType;
    use crate::domain::provenance::Provenance;

    fn prov(source: &str, trigger: &str) -> Provenance {
        Provenance::new(source, trigger)
    }

    fn prov_set(entries: &[(&str, &str)]) -> BTreeSet<Provenance> {
        entries.iter().map(|(s, t)| prov(s, t)).collect()
    }

    #[test]
    fn node_map_merge_adds_new_node() {
        let mut graph = NodeMap::default();
        let delta = NodeMap::new(BTreeMap::from([(
            "node-1".to_string(),
            NodeLattice::new(NodeType::Service, "svc".into(), true, prov_set(&[("a", "t")])),
        )]));
        assert!(graph.merge(delta));
        assert!(graph.as_reveal_ref().contains_key("node-1"));
    }

    #[test]
    fn node_map_merge_grows_provenance() {
        let mut graph = NodeMap::new(BTreeMap::from([(
            "n1".to_string(),
            NodeLattice::new(
                NodeType::Service,
                "svc".into(),
                true,
                prov_set(&[("a", "t1")]),
            ),
        )]));

        let delta = NodeMap::new(BTreeMap::from([(
            "n1".to_string(),
            NodeLattice::new(
                NodeType::Service,
                "svc".into(),
                true,
                prov_set(&[("b", "t2")]),
            ),
        )]));

        assert!(graph.merge(delta));
        let node = graph.as_reveal_ref().get("n1").unwrap();
        assert_eq!(node.provenance.as_reveal_ref().len(), 2);
        assert!(!node.has_conflict());
    }

    #[test]
    fn node_map_merge_detects_conflict() {
        let mut graph = NodeMap::new(BTreeMap::from([(
            "n1".to_string(),
            NodeLattice::new(
                NodeType::Service,
                "svc".into(),
                true,
                prov_set(&[("a", "t")]),
            ),
        )]));

        let delta = NodeMap::new(BTreeMap::from([(
            "n1".to_string(),
            NodeLattice::new(
                NodeType::Infrastructure,
                "svc".into(),
                true,
                prov_set(&[("b", "t")]),
            ),
        )]));

        graph.merge(delta);
        let node = graph.as_reveal_ref().get("n1").unwrap();
        assert!(node.has_conflict());
    }

    #[test]
    fn edge_map_merge_adds_new_edge() {
        let mut graph = EdgeMap::default();
        let key = EdgeKey::new("a", "b", EdgeType::DependsOn);
        let delta = EdgeMap::new(BTreeMap::from([(
            key.clone(),
            EdgeLattice::new(prov_set(&[("agent-1", "t")])),
        )]));
        assert!(graph.merge(delta));
        assert!(graph.as_reveal_ref().contains_key(&key));
    }

    #[test]
    fn edge_map_merge_grows_provenance() {
        let key = EdgeKey::new("a", "b", EdgeType::DependsOn);
        let mut graph = EdgeMap::new(BTreeMap::from([(
            key.clone(),
            EdgeLattice::new(prov_set(&[("agent-1", "t1")])),
        )]));

        let delta = EdgeMap::new(BTreeMap::from([(
            key.clone(),
            EdgeLattice::new(prov_set(&[("agent-2", "t2")])),
        )]));

        assert!(graph.merge(delta));
        let edge = graph.as_reveal_ref().get(&key).unwrap();
        assert_eq!(edge.provenance.as_reveal_ref().len(), 2);
    }
}
