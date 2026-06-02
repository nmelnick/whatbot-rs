//! Console IO: stdin/stdout for development.
//!
//! Implements [`whatbot_core::Io`]. Reads lines from stdin into
//! `RawEvent::Message` and prints `Reply` to stdout. Pretends to be a single
//! channel with a single user.

use async_trait::async_trait;
use chrono::Utc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use whatbot_core::{
    ChannelId, Destination, InboundSender, Io, IoError, IoHandle, OutboundReceiver, RawEvent,
    Reply, ServiceId, Visibility,
};

pub struct ConsoleIo {
    service: ServiceId,
    bot_handle: String,
    user_handle: String,
    channel: ChannelId,
}

impl ConsoleIo {
    pub fn new(bot_handle: impl Into<String>, user_handle: impl Into<String>) -> Self {
        Self {
            service: ServiceId::new("console"),
            bot_handle: bot_handle.into(),
            user_handle: user_handle.into(),
            channel: ChannelId::new("stdio"),
        }
    }

    pub fn with_service(mut self, service: impl Into<String>) -> Self {
        self.service = ServiceId::new(service);
        self
    }
}

#[async_trait]
impl Io for ConsoleIo {
    fn service_id(&self) -> &ServiceId {
        &self.service
    }

    async fn start(self: Box<Self>, inbound: InboundSender) -> Result<IoHandle, IoError> {
        let (outbound_tx, outbound_rx) = mpsc::channel::<Reply>(64);
        let cfg = ConsoleConfig {
            service: self.service.clone(),
            bot_handle: self.bot_handle.clone(),
            user_handle: self.user_handle.clone(),
            channel: self.channel.clone(),
        };
        tokio::spawn(run_inbound(cfg, inbound));
        tokio::spawn(run_outbound(outbound_rx));
        info!(
            service = self.service.as_str(),
            "console io ready (type to chat, ctrl-d to exit)"
        );
        Ok(IoHandle {
            service: self.service.clone(),
            outbound: outbound_tx,
            task: None,
        })
    }
}

#[derive(Clone)]
struct ConsoleConfig {
    service: ServiceId,
    bot_handle: String,
    user_handle: String,
    channel: ChannelId,
}

async fn run_inbound(cfg: ConsoleConfig, inbound: InboundSender) {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    loop {
        match reader.next_line().await {
            Ok(Some(line)) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let raw = RawEvent::Message {
                    service: cfg.service.clone(),
                    channel: cfg.channel.clone(),
                    visibility: Visibility::Public,
                    author_handle: cfg.user_handle.clone(),
                    author_display: cfg.user_handle.clone(),
                    bot_handle: cfg.bot_handle.clone(),
                    bot_display: cfg.bot_handle.clone(),
                    text: line,
                    ts: Utc::now(),
                    addressed_to_bot: true,
                    provider_message_id: None,
                };
                if let Err(e) = inbound.send(raw).await {
                    warn!(?e, "inbound channel closed");
                    break;
                }
            }
            Ok(None) => {
                info!("stdin closed");
                break;
            }
            Err(e) => {
                warn!(?e, "stdin read error");
                break;
            }
        }
    }
}

async fn run_outbound(mut rx: OutboundReceiver) {
    while let Some(reply) = rx.recv().await {
        let prefix = match &reply.destination {
            Destination::Channel { channel, .. } => format!("[{}]", channel.as_str()),
            Destination::Direct { account, .. } => format!("[dm:{}]", account.handle),
            Destination::ReplyTo { channel, .. } => format!("[{}]", channel.as_str()),
        };
        debug!(?reply, "console outbound");
        println!("{prefix} {}", reply.text);
    }
}
