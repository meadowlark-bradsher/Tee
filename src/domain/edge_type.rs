use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EdgeType {
    DependsOn,
    PropagatesTo,
    ManifestsAs,
}

impl std::fmt::Display for EdgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DependsOn => write!(f, "DEPENDS_ON"),
            Self::PropagatesTo => write!(f, "PROPAGATES_TO"),
            Self::ManifestsAs => write!(f, "MANIFESTS_AS"),
        }
    }
}
