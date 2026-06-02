//! IO trait, handle, channel types.
//!
//! An [`Io`] is a chat surface — a connection, console, web site. It runs as
//! one or more Tokio tasks, pushes `RawEvent` into the dispatcher's channel,
//! and consumes [`Reply`](crate::Reply)s from its own channel.

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::context::ServiceId;
use crate::event::RawEvent;
use crate::mentions::{default_mention_renderer, MentionRenderer};
use crate::reply::Reply;

/// Sent by an IO into the dispatcher's inbound channel
pub type InboundSender = mpsc::Sender<RawEvent>;

/// Received by an IO from the dispatcher
pub type OutboundReceiver = mpsc::Receiver<Reply>;
pub type OutboundSender = mpsc::Sender<Reply>;

pub struct IoHandle {
    pub service: ServiceId,
    pub outbound: OutboundSender,
    pub task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Debug, Error)]
#[error("io startup failed: {0}")]
pub struct IoError(pub Box<dyn std::error::Error + Send + Sync>);

impl IoError {
    pub fn new<E: std::error::Error + Send + Sync + 'static>(e: E) -> Self {
        Self(Box::new(e))
    }
}

/// A chat IO. Implementations live in `whatbot-io-*` crates
#[async_trait]
pub trait Io: Send {
    /// Service id this IO claims
    fn service_id(&self) -> &ServiceId;

    /// Service-specific mention renderer
    fn mention_renderer(&self) -> Arc<dyn MentionRenderer> {
        default_mention_renderer()
    }

    /// Start the IO
    async fn start(self: Box<Self>, inbound: InboundSender) -> Result<IoHandle, IoError>;
}
