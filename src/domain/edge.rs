use std::collections::BTreeSet;

use lattices::set_union::SetUnionBTreeSet;
use lattices::{IsBot, LatticeFrom, Merge};
use serde::{Deserialize, Serialize};

use super::edge_type::EdgeType;
use super::provenance::Provenance;

/// Composite identity key for a hypothesis edge: `(source, target, type)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EdgeKey {
    pub source: String,
    pub target: String,
    pub edge_type: EdgeType,
}

impl EdgeKey {
    pub fn new(source: impl Into<String>, target: impl Into<String>, edge_type: EdgeType) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            edge_type,
        }
    }
}

/// Lattice-backed representation of a hypothesis edge's mutable properties.
/// Identity is in `EdgeKey` (the map key). Only provenance is a lattice field.
#[derive(Debug, Clone)]
pub struct EdgeLattice {
    pub provenance: SetUnionBTreeSet<Provenance>,
}

impl EdgeLattice {
    pub fn new(provenance: BTreeSet<Provenance>) -> Self {
        Self {
            provenance: SetUnionBTreeSet::new(provenance),
        }
    }
}

impl Merge<EdgeLattice> for EdgeLattice {
    fn merge(&mut self, other: EdgeLattice) -> bool {
        self.provenance.merge(other.provenance)
    }
}

impl LatticeFrom<EdgeLattice> for EdgeLattice {
    fn lattice_from(other: EdgeLattice) -> Self {
        other
    }
}

impl IsBot for EdgeLattice {
    fn is_bot(&self) -> bool {
        self.provenance.as_reveal_ref().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov(source: &str, trigger: &str) -> Provenance {
        Provenance::new(source, trigger)
    }

    #[test]
    fn merge_accumulates_provenance() {
        let mut a = EdgeLattice::new(BTreeSet::from([prov("a", "t1")]));
        let b = EdgeLattice::new(BTreeSet::from([prov("b", "t2")]));
        assert!(a.merge(b));
        assert_eq!(a.provenance.as_reveal_ref().len(), 2);
    }

    #[test]
    fn merge_idempotent() {
        let mut a = EdgeLattice::new(BTreeSet::from([prov("a", "t1")]));
        let b = EdgeLattice::new(BTreeSet::from([prov("a", "t1")]));
        assert!(!a.merge(b));
        assert_eq!(a.provenance.as_reveal_ref().len(), 1);
    }

    #[test]
    fn merge_commutative() {
        let mut ab = EdgeLattice::new(BTreeSet::from([prov("a", "t1")]));
        ab.merge(EdgeLattice::new(BTreeSet::from([prov("b", "t2")])));

        let mut ba = EdgeLattice::new(BTreeSet::from([prov("b", "t2")]));
        ba.merge(EdgeLattice::new(BTreeSet::from([prov("a", "t1")])));

        assert_eq!(
            ab.provenance.as_reveal_ref(),
            ba.provenance.as_reveal_ref()
        );
    }

    #[test]
    fn edge_key_ordering() {
        let k1 = EdgeKey::new("a", "b", EdgeType::DependsOn);
        let k2 = EdgeKey::new("a", "b", EdgeType::PropagatesTo);
        let k3 = EdgeKey::new("a", "c", EdgeType::DependsOn);
        assert!(k1 < k2);
        assert!(k1 < k3);
    }

    #[test]
    fn empty_edge_is_bot() {
        let e = EdgeLattice::new(BTreeSet::new());
        assert!(e.is_bot());
    }

    #[test]
    fn non_empty_edge_is_not_bot() {
        let e = EdgeLattice::new(BTreeSet::from([prov("a", "t")]));
        assert!(!e.is_bot());
    }
}
