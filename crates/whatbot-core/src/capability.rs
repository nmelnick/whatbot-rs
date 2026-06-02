use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::context::ChannelId;

/// A capability granted to an [`Account`](crate::Account)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Global administrator. Can run admin commands across all services.
    Admin,
    /// Bot owner. Implies `Admin` plus permission to restart, reconfigure.
    Owner,
    /// Moderator of a specific channel.
    Mod(ChannelId),
    /// Free-form capability name for extensions to define.
    Custom(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitySet(HashSet<Capability>);

impl CapabilitySet {
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    /// Build a set from any iterable of capabilities. 
    pub fn from_caps<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }

    pub fn contains(&self, cap: &Capability) -> bool {
        if self.0.contains(cap) {
            return true;
        }
        if matches!(cap, Capability::Admin) && self.0.contains(&Capability::Owner) {
            return true;
        }
        false
    }

    pub fn insert(&mut self, cap: Capability) -> bool {
        self.0.insert(cap)
    }

    pub fn remove(&mut self, cap: &Capability) -> bool {
        self.0.remove(cap)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.0.iter()
    }
}
