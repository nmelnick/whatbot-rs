//! Discord IO via serenity.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serenity::all::{
    ChannelId as DiscordChannelId, CreateMessage, GatewayIntents, Http,
    MessageId as DiscordMessageId, Ready,
};
use serenity::client::{Client, Context as SerenityContext, EventHandler};
use serenity::model::channel::Message as DiscordMessage;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use whatbot_core::{
    Account, ChannelId, Destination, InboundSender, Io, IoError, IoHandle, MentionRenderer,
    OutboundReceiver, RawEvent, Reply, ServiceId, Visibility,
};

#[derive(Clone, Debug)]
struct BotIdentity {
    id: u64,
    name: String,
}

/// Renders Discord mentions as `<@leonard>`.
#[derive(Debug)]
pub struct DiscordMentionRenderer {
    pub my_service: ServiceId,
}

impl MentionRenderer for DiscordMentionRenderer {
    fn render(&self, account: &Account) -> String {
        if account.service == self.my_service
            && !account.handle.is_empty()
            && account.handle.chars().all(|c| c.is_ascii_digit())
        {
            format!("<@{}>", account.handle)
        } else {
            account.display.clone()
        }
    }
}

#[derive(Debug, Error)]
pub enum DiscordIoError {
    #[error("serenity: {0}")]
    Serenity(Box<serenity::Error>),
    #[error("invalid {kind} id `{value}`: {source}")]
    InvalidId {
        kind: &'static str,
        value: String,
        #[source]
        source: std::num::ParseIntError,
    },
}

impl From<serenity::Error> for DiscordIoError {
    fn from(e: serenity::Error) -> Self {
        Self::Serenity(Box::new(e))
    }
}

#[derive(Clone)]
pub struct DiscordConfig {
    pub token: String,
    /// Service id used in RawEvent.
    pub service_id: ServiceId,
    /// Optional name prefix that the bot recognizes as being addressed
    /// directly (e.g. `whatbot:`).
    pub addressed_prefix: Option<String>,
}

impl DiscordConfig {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            service_id: ServiceId::new("discord"),
            addressed_prefix: None,
        }
    }

    pub fn with_addressed_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.addressed_prefix = Some(prefix.into());
        self
    }
}

pub struct DiscordIo {
    config: DiscordConfig,
}

impl DiscordIo {
    pub fn new(config: DiscordConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Io for DiscordIo {
    fn service_id(&self) -> &ServiceId {
        &self.config.service_id
    }

    fn mention_renderer(&self) -> Arc<dyn MentionRenderer> {
        Arc::new(DiscordMentionRenderer {
            my_service: self.config.service_id.clone(),
        })
    }

    async fn start(self: Box<Self>, inbound: InboundSender) -> Result<IoHandle, IoError> {
        let intents = GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::DIRECT_MESSAGES;

        let handler = Handler {
            service_id: self.config.service_id.clone(),
            inbound,
            addressed_prefix: self.config.addressed_prefix.clone(),
            bot_identity: Arc::new(OnceLock::new()),
        };

        let mut client = Client::builder(&self.config.token, intents)
            .event_handler(handler)
            .await
            .map_err(IoError::new)?;

        let http = client.http.clone();
        let (outbound_tx, outbound_rx) = mpsc::channel::<Reply>(64);
        tokio::spawn(run_outbound(http, outbound_rx));

        let gateway = tokio::spawn(async move {
            if let Err(e) = client.start().await {
                warn!(?e, "discord gateway exited");
            }
        });

        info!(
            service = self.config.service_id.as_str(),
            "discord io connected"
        );
        Ok(IoHandle {
            service: self.config.service_id.clone(),
            outbound: outbound_tx,
            task: Some(gateway),
        })
    }
}

struct Handler {
    service_id: ServiceId,
    inbound: InboundSender,
    addressed_prefix: Option<String>,
    bot_identity: Arc<OnceLock<BotIdentity>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: SerenityContext, ready: Ready) {
        let identity = BotIdentity {
            id: ready.user.id.get(),
            name: ready.user.name.clone(),
        };
        info!(bot = %identity.name, id = identity.id, "discord ready");
        let _ = self.bot_identity.set(identity);
    }

    async fn message(&self, ctx: SerenityContext, msg: DiscordMessage) {
        // Ignore our own messages
        if msg.author.bot {
            return;
        }
        let Some(bot) = self.bot_identity.get() else {
            warn!("bot identity not yet cached; dropping early message");
            return;
        };

        let visibility = classify_visibility(&ctx, &msg);
        let text = msg.content.clone();
        let addressed_to_bot = detect_addressed(
            &text,
            bot.id,
            &bot.name,
            self.addressed_prefix.as_deref(),
            matches!(visibility, Visibility::Private),
        );

        let cleaned =
            strip_leading_mention(&text, bot.id, &bot.name, self.addressed_prefix.as_deref())
                .trim()
                .to_string();

        let author_display = msg
            .author
            .global_name
            .clone()
            .unwrap_or_else(|| msg.author.name.clone());

        let raw = RawEvent::Message {
            service: self.service_id.clone(),
            channel: ChannelId::new(msg.channel_id.get().to_string()),
            visibility,
            author_handle: msg.author.id.get().to_string(),
            author_display,
            bot_handle: bot.id.to_string(),
            bot_display: bot.name.clone(),
            text: cleaned,
            ts: chrono::Utc::now(),
            addressed_to_bot,
            provider_message_id: Some(msg.id.get().to_string()),
        };

        if let Err(e) = self.inbound.send(raw).await {
            warn!(?e, "inbound channel closed");
        }
    }
}

/// Public if in a guild, Private if a DM.
fn classify_visibility(_ctx: &SerenityContext, msg: &DiscordMessage) -> Visibility {
    if msg.guild_id.is_none() {
        Visibility::Private
    } else {
        Visibility::Public
    }
}

fn detect_addressed(
    text: &str,
    bot_id: u64,
    bot_name: &str,
    prefix: Option<&str>,
    is_dm: bool,
) -> bool {
    if is_dm {
        return true;
    }
    let mention_a = format!("<@{bot_id}>");
    let mention_b = format!("<@!{bot_id}>");
    if text.starts_with(&mention_a) || text.starts_with(&mention_b) {
        return true;
    }
    if let Some(p) = prefix {
        let lower = text.to_lowercase();
        let p_lower = p.to_lowercase();
        if lower.starts_with(&format!("{p_lower}:")) || lower.starts_with(&format!("{p_lower},")) {
            return true;
        }
    }
    let bot_lower = bot_name.to_lowercase();
    let lower = text.to_lowercase();
    if lower.starts_with(&format!("{bot_lower}:")) || lower.starts_with(&format!("{bot_lower},")) {
        return true;
    }
    false
}

fn strip_leading_mention(
    text: &str,
    bot_id: u64,
    bot_name: &str,
    configured_prefix: Option<&str>,
) -> String {
    let mention_a = format!("<@{bot_id}>");
    let mention_b = format!("<@!{bot_id}>");
    if let Some(rest) = text
        .strip_prefix(&mention_a)
        .or_else(|| text.strip_prefix(&mention_b))
    {
        return rest.trim_start().to_string();
    }
    let lower = text.to_lowercase();
    let mut heads: Vec<String> = vec![bot_name.to_lowercase()];
    if let Some(p) = configured_prefix {
        heads.push(p.to_lowercase());
    }
    for head_base in heads {
        for sep in [":", ","] {
            let head = format!("{head_base}{sep}");
            if lower.starts_with(&head) {
                return text[head.len()..].trim_start().to_string();
            }
        }
    }
    text.to_string()
}

async fn run_outbound(http: Arc<Http>, mut rx: OutboundReceiver) {
    while let Some(reply) = rx.recv().await {
        if let Err(e) = send_one(&http, &reply).await {
            warn!(?e, "discord send failed");
        }
    }
}

async fn send_one(http: &Http, reply: &Reply) -> Result<(), DiscordIoError> {
    match &reply.destination {
        Destination::Channel { channel, .. } => {
            let id = parse_channel_id(channel.as_str())?;
            id.say(http, &reply.text).await?;
        }
        Destination::Direct { account, .. } => {
            let user_id = parse_user_id(&account.handle)?;
            let dm = user_id.create_dm_channel(http).await?;
            dm.id.say(http, &reply.text).await?;
        }
        Destination::ReplyTo {
            channel,
            parent_message_id,
            ..
        } => {
            let id = parse_channel_id(channel.as_str())?;
            let parent = parse_message_id(parent_message_id)?;
            let payload = CreateMessage::new()
                .content(reply.text.clone())
                .reference_message((id, parent));
            id.send_message(http, payload).await?;
        }
    }
    debug!(dest = ?reply.destination, "discord sent");
    Ok(())
}

fn parse_channel_id(s: &str) -> Result<DiscordChannelId, DiscordIoError> {
    s.parse::<u64>()
        .map(DiscordChannelId::new)
        .map_err(|source| DiscordIoError::InvalidId {
            kind: "channel",
            value: s.to_string(),
            source,
        })
}

fn parse_user_id(s: &str) -> Result<serenity::all::UserId, DiscordIoError> {
    s.parse::<u64>()
        .map(serenity::all::UserId::new)
        .map_err(|source| DiscordIoError::InvalidId {
            kind: "user",
            value: s.to_string(),
            source,
        })
}

fn parse_message_id(s: &str) -> Result<DiscordMessageId, DiscordIoError> {
    s.parse::<u64>()
        .map(DiscordMessageId::new)
        .map_err(|source| DiscordIoError::InvalidId {
            kind: "message",
            value: s.to_string(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_renderer_emits_mention_for_native_account() {
        let svc = ServiceId::new("discord");
        let mut nichelle = Account::synthetic(svc.clone(), "123456789");
        nichelle.display = "nichelle".to_string();
        let r = DiscordMentionRenderer { my_service: svc };
        assert_eq!(r.render(&nichelle), "<@123456789>");
    }

    #[test]
    fn discord_renderer_falls_back_for_foreign_service() {
        let mut nichelle = Account::synthetic(ServiceId::new("irc"), "nichelle");
        nichelle.display = "nichelle".to_string();
        let r = DiscordMentionRenderer {
            my_service: ServiceId::new("discord"),
        };
        // Cross-service mention: can't be Discord's `<@...>`; use display.
        assert_eq!(r.render(&nichelle), "nichelle");
    }

    #[test]
    fn discord_renderer_falls_back_for_non_numeric_handle() {
        // Defensive: if someone constructed a Discord account with a
        // non-snowflake handle, don't emit a malformed mention.
        let svc = ServiceId::new("discord");
        let mut nichelle = Account::synthetic(svc.clone(), "nichelle");
        nichelle.display = "nichelle".to_string();
        let r = DiscordMentionRenderer { my_service: svc };
        assert_eq!(r.render(&nichelle), "nichelle");
    }

    #[test]
    fn addressed_via_mention() {
        assert!(detect_addressed(
            "<@123> hello",
            123,
            "whatbot",
            None,
            false
        ));
        assert!(detect_addressed(
            "<@!123> hello",
            123,
            "whatbot",
            None,
            false
        ));
    }

    #[test]
    fn addressed_via_name_prefix() {
        assert!(detect_addressed("whatbot: hi", 9, "whatbot", None, false));
        assert!(detect_addressed("WhatBot, hi", 9, "whatbot", None, false));
    }

    #[test]
    fn addressed_via_configured_prefix() {
        assert!(detect_addressed(
            "bot: hi",
            9,
            "whatbot",
            Some("bot"),
            false
        ));
    }

    #[test]
    fn not_addressed_in_passing_chat() {
        assert!(!detect_addressed(
            "hey whatbot is cool",
            9,
            "whatbot",
            None,
            false
        ));
    }

    #[test]
    fn private_is_always_addressed() {
        assert!(detect_addressed("anything", 9, "whatbot", None, true));
    }

    #[test]
    fn strip_mention_leaves_content() {
        assert_eq!(
            strip_leading_mention("<@123> what is rust", 123, "whatbot", None),
            "what is rust"
        );
        assert_eq!(
            strip_leading_mention("whatbot: what is rust", 123, "whatbot", None),
            "what is rust"
        );
        assert_eq!(
            strip_leading_mention("<@!123> what is rust", 123, "whatbot", None),
            "what is rust"
        );
    }

    #[test]
    fn strip_mention_noop_when_absent() {
        assert_eq!(
            strip_leading_mention("hello there", 123, "whatbot", None),
            "hello there"
        );
    }

    #[test]
    fn strip_mention_handles_configured_prefix() {
        // Bug from review: the addressed_prefix used to be detected but not
        // stripped, so commands saw the raw `wb: ...` text and nothing matched.
        assert_eq!(
            strip_leading_mention("wb: what is rust", 123, "whatbot", Some("wb")),
            "what is rust"
        );
        assert_eq!(
            strip_leading_mention("WB, what is rust", 123, "whatbot", Some("wb")),
            "what is rust"
        );
    }

    #[test]
    fn strip_mention_prefers_bot_name_then_falls_back_to_prefix() {
        assert_eq!(
            strip_leading_mention("whatbot: hi", 123, "whatbot", Some("wb")),
            "hi"
        );
        assert_eq!(
            strip_leading_mention("wb: hi", 123, "whatbot", Some("wb")),
            "hi"
        );
    }
}
