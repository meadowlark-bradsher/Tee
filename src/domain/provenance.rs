use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// A provenance record tracking who triggered an operation and when.
///
/// Identity is `(source, trigger)` only — the timestamp is informational metadata
/// excluded from equality, ordering, and hashing. This means:
/// - A `BTreeSet<Provenance>` deduplicates by `(source, trigger)`
/// - `BTreeSet::insert` keeps the existing entry on collision (first-write-wins on timestamp)
/// - `SetUnion::merge` uses `Extend`, which calls `insert`, preserving first-write-wins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub source: String,
    pub trigger: String,
    /// Informational only. Not part of identity.
    /// Stored as (seconds, nanos) from epoch to avoid prost_types dependency in domain layer.
    pub timestamp_seconds: i64,
    pub timestamp_nanos: i32,
}

impl Provenance {
    pub fn new(source: impl Into<String>, trigger: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            trigger: trigger.into(),
            timestamp_seconds: 0,
            timestamp_nanos: 0,
        }
    }

    pub fn with_timestamp(mut self, seconds: i64, nanos: i32) -> Self {
        self.timestamp_seconds = seconds;
        self.timestamp_nanos = nanos;
        self
    }
}

// Identity is (source, trigger) only — timestamp excluded.

impl PartialEq for Provenance {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && self.trigger == other.trigger
    }
}
impl Eq for Provenance {}

impl PartialOrd for Provenance {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Provenance {
    fn cmp(&self, other: &Self) -> Ordering {
        (&self.source, &self.trigger).cmp(&(&other.source, &other.trigger))
    }
}

impl std::hash::Hash for Provenance {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.source.hash(state);
        self.trigger.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn equality_ignores_timestamp() {
        let a = Provenance::new("agent-1", "alert").with_timestamp(100, 0);
        let b = Provenance::new("agent-1", "alert").with_timestamp(200, 0);
        assert_eq!(a, b);
    }

    #[test]
    fn different_source_not_equal() {
        let a = Provenance::new("agent-1", "alert");
        let b = Provenance::new("agent-2", "alert");
        assert_ne!(a, b);
    }

    #[test]
    fn different_trigger_not_equal() {
        let a = Provenance::new("agent-1", "alert");
        let b = Provenance::new("agent-1", "log-scan");
        assert_ne!(a, b);
    }

    #[test]
    fn btreeset_dedup_by_identity() {
        let a = Provenance::new("agent-1", "alert").with_timestamp(100, 0);
        let b = Provenance::new("agent-1", "alert").with_timestamp(200, 0);

        let mut set = BTreeSet::new();
        set.insert(a);
        set.insert(b);

        assert_eq!(set.len(), 1);
        // First-write-wins: timestamp should be from first insert (100)
        let entry = set.iter().next().unwrap();
        assert_eq!(entry.timestamp_seconds, 100);
    }

    #[test]
    fn btreeset_keeps_distinct_entries() {
        let a = Provenance::new("agent-1", "alert");
        let b = Provenance::new("agent-2", "log-scan");

        let mut set = BTreeSet::new();
        set.insert(a);
        set.insert(b);

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn ordering_is_deterministic() {
        let a = Provenance::new("a", "x");
        let b = Provenance::new("a", "y");
        let c = Provenance::new("b", "x");

        assert!(a < b); // same source, trigger "x" < "y"
        assert!(b < c); // source "a" < "b"
    }
}
