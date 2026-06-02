use std::any::Any;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::capability::Capability;
use crate::context::Context;
use crate::event::Event;
use crate::reply::Reply;
use crate::state::StateSlot;

/// Priority tier for command dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Priority {
    /// Always runs
    Primary,
    /// Always runs, after Primary
    Core,
    /// Skipped if handled earlier
    Extension,
    /// Skipped if hhandled earlier
    Last,
}

impl Priority {
    pub const ALL: [Priority; 4] = [
        Priority::Primary,
        Priority::Core,
        Priority::Extension,
        Priority::Last,
    ];
}

/// Static metadata about a command. Returned by [`Command::meta`].
#[derive(Debug, Clone)]
pub struct CommandMeta {
    pub name: &'static str,
    pub priority: Priority,
    pub require_direct: bool,
    pub required_caps: Vec<Capability>,
    pub help: &'static str,
}

impl CommandMeta {
    pub const fn new(name: &'static str, priority: Priority, help: &'static str) -> Self {
        Self {
            name,
            priority,
            require_direct: false,
            required_caps: Vec::new(),
            help,
        }
    }

    pub const fn primary(name: &'static str, help: &'static str) -> Self {
        Self::new(name, Priority::Primary, help)
    }
    pub const fn core(name: &'static str, help: &'static str) -> Self {
        Self::new(name, Priority::Core, help)
    }
    pub const fn extension(name: &'static str, help: &'static str) -> Self {
        Self::new(name, Priority::Extension, help)
    }
    pub const fn last_resort(name: &'static str, help: &'static str) -> Self {
        Self::new(name, Priority::Last, help)
    }

    /// Require the bot to have been addressed by mention/name/DM.
    pub fn require_direct(mut self) -> Self {
        self.require_direct = true;
        self
    }

    /// Require the sender to hold the given capability.
    pub fn require_cap(mut self, cap: Capability) -> Self {
        self.required_caps.push(cap);
        self
    }

    /// Does this context have permission to invoke this command
    pub fn permits(&self, ctx: &Context) -> bool {
        if self.require_direct && !ctx.addressed_to_bot {
            return false;
        }
        for cap in &self.required_caps {
            if !ctx.has(cap) {
                return false;
            }
        }
        true
    }
}

/// Unwrap a [`MatchData`] back into the concrete type a command's [`matches`]
/// put in it
#[macro_export]
macro_rules! match_data {
    ($m:expr => $t:ty) => {
        match $m.downcast::<$t>() {
            Ok(boxed) => *boxed,
            Err(_) => return $crate::CommandResult::empty(),
        }
    };
}

/// Match result, cast by the consumer
pub struct MatchData(pub Box<dyn Any + Send>);

impl MatchData {
    pub fn new<T: Any + Send>(value: T) -> Self {
        Self(Box::new(value))
    }
    pub fn downcast<T: Any + Send>(self) -> Result<Box<T>, Self> {
        self.0.downcast::<T>().map_err(MatchData)
    }
}

impl std::fmt::Debug for MatchData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MatchData(..)")
    }
}

#[derive(Debug, Default)]
pub struct CommandResult {
    pub replies: Vec<Reply>,
    /// If true, dispatcher stops processing further commands in this tier.
    pub stop: bool,
    /// True when the command considers itself to have handled the message even
    /// without producing a reply. Useful for commands that swallow a message
    /// silently.
    pub consumed: bool,
}

impl CommandResult {
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn reply(reply: Reply) -> Self {
        Self {
            replies: vec![reply],
            stop: false,
            consumed: false,
        }
    }
    pub fn stop() -> Self {
        Self {
            replies: Vec::new(),
            stop: true,
            consumed: false,
        }
    }
    pub fn handled_silently() -> Self {
        Self {
            replies: Vec::new(),
            stop: true,
            consumed: true,
        }
    }
    pub fn with_stop(mut self, stop: bool) -> Self {
        self.stop = stop;
        self
    }
    pub fn with_consumed(mut self, consumed: bool) -> Self {
        self.consumed = consumed;
        self
    }
}

/// The Command trait. Implementations live in `whatbot-commands`.
#[async_trait]
pub trait Command: Send + Sync {
    fn meta(&self) -> &CommandMeta;

    /// Sync, does this command want to handle this event?
    /// Returns [`MatchData`] which is passed back to [`Command::handle`].
    fn matches(&self, evt: &Event, ctx: &Context) -> Option<MatchData>;

    /// Async, may do I/O. Only invoked if [`Command::matches`] returned `Some`.
    async fn handle(&self, m: MatchData, ctx: &Context, state: &mut StateSlot) -> CommandResult;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ChannelId, ServiceId, Visibility};
    use crate::identity::Account;

    fn ctx(addressed: bool, caps: Vec<Capability>) -> Context {
        let service = ServiceId::new("test");
        let mut author = Account::synthetic(service.clone(), "nichelle");
        for c in caps {
            author.capabilities.insert(c);
        }
        Context {
            service: service.clone(),
            channel: ChannelId::new("main"),
            visibility: Visibility::Public,
            author,
            bot: Account::synthetic(service, "whatbot"),
            addressed_to_bot: addressed,
            mention_renderer: crate::mentions::default_mention_renderer(),
        }
    }

    fn meta(require_direct: bool, required_caps: Vec<Capability>) -> CommandMeta {
        CommandMeta {
            name: "test",
            priority: Priority::Core,
            require_direct,
            required_caps,
            help: "",
        }
    }

    #[test]
    fn permits_when_no_constraints() {
        assert!(meta(false, vec![]).permits(&ctx(false, vec![])));
    }

    #[test]
    fn require_direct_blocks_undirected_messages() {
        assert!(!meta(true, vec![]).permits(&ctx(false, vec![])));
    }

    #[test]
    fn require_direct_allows_directed_messages() {
        assert!(meta(true, vec![]).permits(&ctx(true, vec![])));
    }

    #[test]
    fn required_caps_block_when_missing() {
        let m = meta(false, vec![Capability::Admin]);
        assert!(!m.permits(&ctx(false, vec![])));
    }

    #[test]
    fn required_caps_pass_when_granted() {
        let m = meta(false, vec![Capability::Admin]);
        assert!(m.permits(&ctx(false, vec![Capability::Admin])));
    }

    #[test]
    fn require_direct_and_caps_both_enforced() {
        let m = meta(true, vec![Capability::Admin]);
        // Directed but no caps: deny.
        assert!(!m.permits(&ctx(true, vec![])));
        // Caps but not directed: deny.
        assert!(!m.permits(&ctx(false, vec![Capability::Admin])));
        // Both satisfied: allow.
        assert!(m.permits(&ctx(true, vec![Capability::Admin])));
    }
}
