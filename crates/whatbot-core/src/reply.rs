use crate::context::{ChannelId, ServiceId};
use crate::identity::Account;

/// An outbound reply produced by a Command
#[derive(Debug, Clone)]
pub struct Reply {
    pub destination: Destination,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum Destination {
    /// Public reply into a channel
    Channel {
        service: ServiceId,
        channel: ChannelId,
    },
    /// Direct message to a specific account
    Direct {
        service: ServiceId,
        account: Account,
    },
    /// Threaded/inline reply to a specific source message
    ReplyTo {
        service: ServiceId,
        channel: ChannelId,
        parent_message_id: String,
    },
}

impl Destination {
    pub fn service(&self) -> &ServiceId {
        match self {
            Destination::Channel { service, .. }
            | Destination::Direct { service, .. }
            | Destination::ReplyTo { service, .. } => service,
        }
    }
}
