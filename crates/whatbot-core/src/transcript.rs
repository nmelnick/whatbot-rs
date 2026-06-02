//! Conversation transcript

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

use crate::context::{ChannelId, ServiceId, Visibility};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone)]
pub struct TranscriptEntry {
    pub ts: DateTime<Utc>,
    pub direction: Direction,
    pub service: ServiceId,
    pub channel: ChannelId,
    pub visibility: Visibility,
    pub speaker: String,
    pub text: String,
}

#[derive(Clone)]
pub struct TranscriptHandle {
    tx: mpsc::Sender<TranscriptEntry>,
}

impl TranscriptHandle {
    pub fn channel(capacity: usize) -> (Self, mpsc::Receiver<TranscriptEntry>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Self { tx }, rx)
    }

    pub fn record(&self, entry: TranscriptEntry) {
        if let Err(e) = self.tx.try_send(entry) {
            tracing::debug!(?e, "transcript channel full or closed; dropping entry");
        }
    }
}
