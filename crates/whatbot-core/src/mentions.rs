//! Service-aware mention rendering

use std::fmt::Debug;
use std::sync::Arc;

use once_cell::sync::Lazy;

use crate::identity::Account;

pub trait MentionRenderer: Send + Sync + Debug {
    fn render(&self, account: &Account) -> String;
}

/// Default returns the account's display name
#[derive(Debug, Default)]
pub struct DisplayNameRenderer;

impl MentionRenderer for DisplayNameRenderer {
    fn render(&self, account: &Account) -> String {
        account.display.clone()
    }
}

static DEFAULT: Lazy<Arc<dyn MentionRenderer>> = Lazy::new(|| Arc::new(DisplayNameRenderer));

pub fn default_mention_renderer() -> Arc<dyn MentionRenderer> {
    DEFAULT.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ServiceId;

    #[test]
    fn display_name_renderer_returns_display() {
        let mut nichelle = Account::synthetic(ServiceId::new("svc"), "nichelle-handle");
        nichelle.display = "nichelle".to_string();
        assert_eq!(DisplayNameRenderer.render(&nichelle), "nichelle");
    }
}
