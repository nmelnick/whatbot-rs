use std::time::Instant;

use crate::context::{ChannelId, Context, ServiceId, Visibility};
use crate::message::Message;

/// A typed, post-resolution event delivered to commands.
#[derive(Debug, Clone)]
pub enum Event {
    Message(Message),
    Joined {
        ctx: Context,
        who: crate::identity::Account,
    },
    Left {
        ctx: Context,
        who: crate::identity::Account,
    },
    Topic {
        ctx: Context,
        topic: String,
        by: crate::identity::Account,
    },
    Tick(Instant),
}

impl Event {
    pub fn context(&self) -> Option<&Context> {
        match self {
            Event::Message(m) => Some(&m.ctx),
            Event::Joined { ctx, .. } | Event::Left { ctx, .. } | Event::Topic { ctx, .. } => {
                Some(ctx)
            }
            Event::Tick(_) => None,
        }
    }
}

/// A pre-resolution event emitted by an IO
#[derive(Debug, Clone)]
pub enum RawEvent {
    Message {
        service: ServiceId,
        channel: ChannelId,
        visibility: Visibility,
        author_handle: String,
        author_display: String,
        bot_handle: String,
        bot_display: String,
        text: String,
        ts: chrono::DateTime<chrono::Utc>,
        addressed_to_bot: bool,
        provider_message_id: Option<String>,
    },
    Joined {
        service: ServiceId,
        channel: ChannelId,
        who_handle: String,
        who_display: String,
    },
    Left {
        service: ServiceId,
        channel: ChannelId,
        who_handle: String,
        who_display: String,
    },
    Topic {
        service: ServiceId,
        channel: ChannelId,
        topic: String,
        by_handle: String,
        by_display: String,
    },
}
