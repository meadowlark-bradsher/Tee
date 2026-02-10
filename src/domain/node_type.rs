use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NodeType {
    Service,
    Dependency,
    Infrastructure,
    Mechanism,
}

impl std::fmt::Display for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Service => write!(f, "SERVICE"),
            Self::Dependency => write!(f, "DEPENDENCY"),
            Self::Infrastructure => write!(f, "INFRASTRUCTURE"),
            Self::Mechanism => write!(f, "MECHANISM"),
        }
    }
}
