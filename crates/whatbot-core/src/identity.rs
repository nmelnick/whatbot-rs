use serde::{Deserialize, Serialize};

use crate::capability::CapabilitySet;
use crate::context::ServiceId;

/// A per-service identity. Resolved by the dispatcher before any command runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub service: ServiceId,
    pub handle: String,
    pub display: String,
    pub person_id: Option<i64>,
    #[serde(default)]
    pub capabilities: CapabilitySet,
}

impl Account {
    /// Construct an in-memory Account without a database row
    pub fn synthetic(service: ServiceId, handle: impl Into<String>) -> Self {
        let handle = handle.into();
        Self {
            id: 0,
            service,
            display: handle.clone(),
            handle,
            person_id: None,
            capabilities: CapabilitySet::new(),
        }
    }

    /// Case-insensitive comparison against a raw handle string
    pub fn matches_handle(&self, s: &str) -> bool {
        self.handle.eq_ignore_ascii_case(s)
    }

    /// True if two accounts refer to the same identity
    pub fn is_same(&self, other: &Account) -> bool {
        self.service == other.service && self.matches_handle(&other.handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_handle_is_case_insensitive() {
        let a = Account::synthetic(ServiceId::new("test"), "nichelle");
        assert!(a.matches_handle("nichelle"));
        assert!(a.matches_handle("Nichelle"));
        assert!(a.matches_handle("nichELLE"));
        assert!(!a.matches_handle("bob"));
    }

    #[test]
    fn is_same_requires_service_match() {
        let a = Account::synthetic(ServiceId::new("svc-one"), "nichelle");
        let same_svc = Account::synthetic(ServiceId::new("svc-one"), "Nichelle");
        let other_svc = Account::synthetic(ServiceId::new("svc-two"), "nichelle");
        assert!(
            a.is_same(&same_svc),
            "same service + case-insensitive match"
        );
        assert!(
            !a.is_same(&other_svc),
            "different services must not be considered the same identity"
        );
    }
}
