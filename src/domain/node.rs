use std::collections::BTreeSet;

use lattices::set_union::SetUnionBTreeSet;
use lattices::{Conflict, IsBot, IsTop, LatticeFrom, Merge, Min};

use super::node_type::NodeType;
use super::provenance::Provenance;

/// Lattice-backed representation of a hypothesis node's mutable properties.
///
/// The node's `id` is the key in the enclosing map, not stored here.
///
/// Field merge semantics:
/// - `node_type`: `Conflict<NodeType>` — first-write-wins, `is_top()` = conflict detected
/// - `label`: `Conflict<String>` — first-write-wins, `is_top()` = conflict detected
/// - `hypothetical`: `Min<bool>` — once false (confirmed), stays false
/// - `provenance`: `SetUnion<BTreeSet<Provenance>>` — append-only, dedup by `(source, trigger)`
///
/// Note: The README describes `hypothetical` as `Max<bool>`, but the intended semantics
/// ("once false, stays false") are AND/Min. `Min<bool>::default()` = `true` (new nodes
/// start hypothetical). `Min::merge` keeps the minimum: `merge(true, false) → false`.
#[derive(Debug, Clone)]
pub struct NodeLattice {
    pub node_type: Conflict<NodeType>,
    pub label: Conflict<String>,
    pub hypothetical: Min<bool>,
    pub provenance: SetUnionBTreeSet<Provenance>,
}

impl NodeLattice {
    pub fn new(
        node_type: NodeType,
        label: String,
        hypothetical: bool,
        provenance: BTreeSet<Provenance>,
    ) -> Self {
        Self {
            node_type: Conflict::new_from(node_type),
            label: Conflict::new_from(label),
            hypothetical: Min::new(hypothetical),
            provenance: SetUnionBTreeSet::new(provenance),
        }
    }

    /// Returns true if either structural field (type or label) is in conflict state.
    /// A conflict means two agents proposed different values for the same node ID.
    pub fn has_conflict(&self) -> bool {
        self.node_type.is_top() || self.label.is_top()
    }

    /// Returns the conflict details if any structural field is conflicted.
    /// Used to build `MergeConflict` proto responses.
    pub fn conflict_field(&self) -> Option<&'static str> {
        if self.node_type.is_top() {
            Some("type")
        } else if self.label.is_top() {
            Some("label")
        } else {
            None
        }
    }
}

impl Merge<NodeLattice> for NodeLattice {
    fn merge(&mut self, other: NodeLattice) -> bool {
        let mut changed = false;
        changed |= self.node_type.merge(other.node_type);
        changed |= self.label.merge(other.label);
        changed |= self.hypothetical.merge(other.hypothetical);
        changed |= self.provenance.merge(other.provenance);
        changed
    }
}

impl LatticeFrom<NodeLattice> for NodeLattice {
    fn lattice_from(other: NodeLattice) -> Self {
        other
    }
}

impl IsBot for NodeLattice {
    fn is_bot(&self) -> bool {
        // A node with any content is never bottom.
        // Conflict::is_bot() is always false, so this is always false.
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov(source: &str, trigger: &str) -> Provenance {
        Provenance::new(source, trigger)
    }

    fn prov_set(entries: &[(&str, &str)]) -> BTreeSet<Provenance> {
        entries.iter().map(|(s, t)| prov(s, t)).collect()
    }

    // --- Idempotence ---

    #[test]
    fn merge_idempotent_no_change() {
        let mut a = NodeLattice::new(
            NodeType::Service,
            "api-gateway".into(),
            true,
            prov_set(&[("agent-1", "alert-fired")]),
        );
        let b = NodeLattice::new(
            NodeType::Service,
            "api-gateway".into(),
            true,
            prov_set(&[("agent-1", "alert-fired")]),
        );
        assert!(!a.merge(b), "merging identical data should return false");
        assert!(!a.has_conflict());
    }

    // --- Commutativity ---

    #[test]
    fn merge_commutative_provenance() {
        let make = |entries: &[(&str, &str)]| {
            NodeLattice::new(NodeType::Service, "svc".into(), true, prov_set(entries))
        };

        let mut ab = make(&[("agent-1", "alert")]);
        let b = make(&[("agent-2", "log-scan")]);
        ab.merge(b);

        let mut ba = make(&[("agent-2", "log-scan")]);
        let a = make(&[("agent-1", "alert")]);
        ba.merge(a);

        assert_eq!(
            ab.provenance.as_reveal_ref(),
            ba.provenance.as_reveal_ref(),
            "provenance sets should be identical regardless of merge order"
        );
    }

    // --- Conflict detection ---

    #[test]
    fn type_conflict_detected() {
        let mut a = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true,
            prov_set(&[("a", "t")]),
        );
        let b = NodeLattice::new(
            NodeType::Infrastructure,
            "svc".into(),
            true,
            prov_set(&[("a", "t")]),
        );
        a.merge(b);
        assert!(a.has_conflict());
        assert!(a.node_type.is_top());
        assert_eq!(a.conflict_field(), Some("type"));
    }

    #[test]
    fn label_conflict_detected() {
        let mut a = NodeLattice::new(
            NodeType::Service,
            "api-gw".into(),
            true,
            prov_set(&[("a", "t")]),
        );
        let b = NodeLattice::new(
            NodeType::Service,
            "api-gateway".into(),
            true,
            prov_set(&[("a", "t")]),
        );
        a.merge(b);
        assert!(a.has_conflict());
        assert!(a.label.is_top());
        assert_eq!(a.conflict_field(), Some("label"));
    }

    #[test]
    fn no_conflict_same_values() {
        let mut a = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true,
            prov_set(&[("a", "t1")]),
        );
        let b = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true,
            prov_set(&[("b", "t2")]),
        );
        a.merge(b);
        assert!(!a.has_conflict());
        assert_eq!(a.provenance.as_reveal_ref().len(), 2);
    }

    // --- Hypothetical monotonicity (Min<bool>) ---

    #[test]
    fn hypothetical_once_false_stays_false() {
        let mut a = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            false, // confirmed
            prov_set(&[("a", "t")]),
        );
        let b = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true, // try to make hypothetical again
            prov_set(&[("a", "t")]),
        );
        assert!(!a.merge(b), "merge(false, true) should be no-op for Min<bool>");
        assert!(!*a.hypothetical.as_reveal_ref());
    }

    #[test]
    fn hypothetical_confirmed_by_merge() {
        let mut a = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true,
            prov_set(&[("a", "t")]),
        );
        let b = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            false,
            prov_set(&[("a", "t")]),
        );
        assert!(a.merge(b), "merge(true, false) should change to false");
        assert!(!*a.hypothetical.as_reveal_ref());
    }

    #[test]
    fn hypothetical_default_is_true() {
        let h = Min::<bool>::default();
        assert!(*h.as_reveal_ref(), "Min<bool>::default should be true");
    }

    // --- Provenance accumulation ---

    #[test]
    fn provenance_accumulates() {
        let mut a = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true,
            prov_set(&[("agent-1", "t1")]),
        );
        let b = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true,
            prov_set(&[("agent-2", "t2")]),
        );
        a.merge(b);
        assert_eq!(a.provenance.as_reveal_ref().len(), 2);
    }

    #[test]
    fn provenance_dedup_same_identity() {
        let mut a = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true,
            prov_set(&[("agent-1", "alert")]),
        );
        let b = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true,
            prov_set(&[("agent-1", "alert")]),
        );
        assert!(!a.merge(b));
        assert_eq!(a.provenance.as_reveal_ref().len(), 1);
    }

    // --- IsBot ---

    #[test]
    fn node_is_never_bot() {
        let n = NodeLattice::new(
            NodeType::Service,
            "svc".into(),
            true,
            prov_set(&[("a", "t")]),
        );
        assert!(!n.is_bot());
    }
}
