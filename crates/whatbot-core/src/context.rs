use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::capability::Capability;
use crate::command::CommandResult;
use crate::identity::Account;
use crate::mentions::MentionRenderer;
use crate::reply::{Destination, Reply};

/// Identifies a chat service instance
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceId(pub String);

impl ServiceId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Service-scoped channel id
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub String);

impl ChannelId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
    Thread { parent: ChannelId },
}

/// Everything a [`Command`](crate::Command) needs to know about where and who.
#[derive(Debug, Clone)]
pub struct Context {
    pub service: ServiceId,
    pub channel: ChannelId,
    pub visibility: Visibility,
    pub author: Account,
    pub bot: Account,
    pub addressed_to_bot: bool,
    pub mention_renderer: Arc<dyn MentionRenderer>,
}

impl Context {
    /// Key used for per-context scratch state in the [`StateMap`](crate::StateMap).
    pub fn state_key(&self) -> (ServiceId, ChannelId) {
        (self.service.clone(), self.channel.clone())
    }

    /// Render a mention of the given account
    pub fn mention(&self, who: &Account) -> String {
        self.mention_renderer.render(who)
    }

    pub fn is_private(&self) -> bool {
        matches!(self.visibility, Visibility::Private)
    }

    /// Reply into the channel this context represents
    pub fn reply_here(&self, text: impl Into<String>) -> Reply {
        Reply {
            destination: Destination::Channel {
                service: self.service.clone(),
                channel: self.channel.clone(),
            },
            text: text.into(),
        }
    }

    /// DM the author, regardless of where the inbound message arrived
    pub fn reply_direct(&self, text: impl Into<String>) -> Reply {
        Reply {
            destination: Destination::Direct {
                service: self.service.clone(),
                account: self.author.clone(),
            },
            text: text.into(),
        }
    }

    /// Shorthand for `CommandResult::reply(self.reply_here(text))`
    pub fn say(&self, text: impl Into<String>) -> CommandResult {
        CommandResult::reply(self.reply_here(text))
    }

    /// True if `ctx.author` holds the given capability
    pub fn has(&self, cap: &Capability) -> bool {
        self.author.capabilities.contains(cap)
    }
}
