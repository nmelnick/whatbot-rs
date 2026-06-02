use std::ops::Range;

use chrono::{DateTime, Utc};
use url::Url;

use crate::context::Context;
use crate::identity::Account;
use crate::reply::{Destination, Reply};

#[derive(Debug, Clone)]
pub struct MentionRef {
    pub account: Account,
    pub span: Option<Range<usize>>,
}

#[derive(Debug, Clone)]
pub struct LinkRef {
    pub url: Url,
    pub span: Option<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    File,
    Other,
}

#[derive(Debug, Clone)]
pub struct MediaRef {
    pub kind: MediaKind,
    pub url: Url,
    pub mime: Option<String>,
}

/// An inbound message as seen by a Command
#[derive(Debug, Clone)]
pub struct Message {
    pub ctx: Context,
    /// Where most matching comes from
    pub text: String,
    /// If the IO supports it, a list of user mentions
    pub mentions: Vec<MentionRef>,
    /// If the IO supports it, a list of link attachments
    pub links: Vec<LinkRef>,
    /// If the IO supports it, a list of media attachments
    pub media: Vec<MediaRef>,
    /// Timestamp of the message
    pub ts: DateTime<Utc>,
    /// Service-native id, when available, for attaching replies or threads
    pub provider_message_id: Option<String>,
}

impl Message {
    /// Threaded reply to this specific message
    pub fn reply_inline(&self, text: impl Into<String>) -> Reply {
        match &self.provider_message_id {
            Some(parent) => Reply {
                destination: Destination::ReplyTo {
                    service: self.ctx.service.clone(),
                    channel: self.ctx.channel.clone(),
                    parent_message_id: parent.clone(),
                },
                text: text.into(),
            },
            None => self.ctx.reply_here(text),
        }
    }
}
