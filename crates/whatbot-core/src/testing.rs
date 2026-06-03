//! Lightweight harness for testing [`Command`] implementations.
//!
//! Construct a [`CommandTester`], optionally chain configuration (author
//! handle, channel, visibility, capabilities), then call
//! [`CommandTester::send`] to exercise a command and collect any replies.

use std::sync::Arc;

use chrono::Utc;

use crate::capability::Capability;
use crate::command::Command;
use crate::context::{ChannelId, Context, ServiceId, Visibility};
use crate::event::Event;
use crate::identity::Account;
use crate::mentions::{default_mention_renderer, MentionRenderer};
use crate::message::Message;
use crate::monitor::Monitor;
use crate::reply::Reply;
use crate::state::StateMap;

/// A reusable builder for unit-testing a single command in isolation.
///
/// Defaults: `service = "test"`, `channel = "main"`, `author = "nichelle"`,
/// `bot = "whatbot"`, `visibility = Public`, `addressed_to_bot = true`.
/// Override what you need via the builder methods; everything else falls
/// through to a sensible default.
pub struct CommandTester {
    service: ServiceId,
    channel: ChannelId,
    visibility: Visibility,
    author: Account,
    bot: Account,
    addressed_to_bot: bool,
    state: StateMap,
    mention_renderer: Arc<dyn MentionRenderer>,
}

impl Default for CommandTester {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandTester {
    pub fn new() -> Self {
        let service = ServiceId::new("test");
        Self {
            service: service.clone(),
            channel: ChannelId::new("main"),
            visibility: Visibility::Public,
            author: Account::synthetic(service.clone(), "nichelle"),
            bot: Account::synthetic(service, "whatbot"),
            addressed_to_bot: true,
            state: StateMap::new(),
            mention_renderer: default_mention_renderer(),
        }
    }

    pub fn with_mention_renderer<R: MentionRenderer + 'static>(mut self, r: R) -> Self {
        self.mention_renderer = Arc::new(r);
        self
    }

    pub fn with_service(mut self, service: &str) -> Self {
        self.service = ServiceId::new(service);
        self.author.service = self.service.clone();
        self.bot.service = self.service.clone();
        self
    }

    pub fn with_channel(mut self, channel: &str) -> Self {
        self.channel = ChannelId::new(channel);
        self
    }

    pub fn with_author(mut self, handle: &str) -> Self {
        self.author = Account::synthetic(self.service.clone(), handle);
        self
    }

    /// Use a fully-formed [`Account`] for the author — typically one
    /// produced by an `AccountRepo::upsert` in a test setup so the row
    /// exists in the database for FK references.
    pub fn with_author_account(mut self, account: Account) -> Self {
        self.author = account;
        self
    }

    pub fn with_bot(mut self, handle: &str) -> Self {
        self.bot = Account::synthetic(self.service.clone(), handle);
        self
    }

    pub fn private(mut self) -> Self {
        self.visibility = Visibility::Private;
        self.channel = ChannelId::new(format!("dm-{}", self.author.handle));
        self
    }

    pub fn addressed(mut self, addressed: bool) -> Self {
        self.addressed_to_bot = addressed;
        self
    }

    pub fn grant(mut self, cap: Capability) -> Self {
        self.author.capabilities.insert(cap);
        self
    }

    /// Return a tester pointing at a different channel but **sharing the
    /// same `StateMap`** as `self`. Use this to validate per-(service,
    /// channel) state isolation: state set via `self` must not leak to
    /// the returned tester (different channel key) and vice versa.
    ///
    /// Plain `with_channel` on a freshly-`new`'d tester creates a tester
    /// with its own state map — which makes "per-context" tests pass
    /// trivially without exercising the slot keying.
    pub fn fork_with_channel(&self, channel: &str) -> Self {
        Self {
            service: self.service.clone(),
            channel: ChannelId::new(channel),
            visibility: self.visibility.clone(),
            author: self.author.clone(),
            bot: self.bot.clone(),
            addressed_to_bot: self.addressed_to_bot,
            state: self.state.clone(),
            mention_renderer: self.mention_renderer.clone(),
        }
    }

    /// Return a tester pointing at a different service (and reset author/bot
    /// service ids to match) but sharing this tester's `StateMap`. Useful for
    /// verifying that `(service, channel)` keying isolates services even
    /// when channel names collide.
    pub fn fork_with_service(&self, service: &str) -> Self {
        let svc = ServiceId::new(service);
        let mut author = self.author.clone();
        let mut bot = self.bot.clone();
        author.service = svc.clone();
        bot.service = svc.clone();
        Self {
            service: svc,
            channel: self.channel.clone(),
            visibility: self.visibility.clone(),
            author,
            bot,
            addressed_to_bot: self.addressed_to_bot,
            state: self.state.clone(),
            mention_renderer: self.mention_renderer.clone(),
        }
    }

    /// Get a fresh `Context` with the current configuration.
    pub fn context(&self) -> Context {
        Context {
            service: self.service.clone(),
            channel: self.channel.clone(),
            visibility: self.visibility.clone(),
            author: self.author.clone(),
            bot: self.bot.clone(),
            addressed_to_bot: self.addressed_to_bot,
            mention_renderer: self.mention_renderer.clone(),
        }
    }

    /// Build a [`Message`] event with the given text.
    pub fn message(&self, text: impl Into<String>) -> Message {
        Message {
            ctx: self.context(),
            text: text.into(),
            mentions: Vec::new(),
            links: Vec::new(),
            media: Vec::new(),
            ts: Utc::now(),
            provider_message_id: None,
        }
    }

    /// Drive a command through `permits` → `matches` → `handle` and
    /// return a [`DispatchOutcome`] that distinguishes "denied by
    /// capability/require_direct" from "command saw the message but
    /// chose not to handle it" from "command produced replies."
    pub async fn send(&self, cmd: &dyn Command, text: &str) -> DispatchOutcome {
        let msg = self.message(text);
        let ctx = msg.ctx.clone();
        if !cmd.meta().permits(&ctx) {
            return DispatchOutcome::Denied;
        }
        let evt = Event::Message(msg);
        let Some(m) = cmd.matches(&evt, &ctx) else {
            return DispatchOutcome::NoMatch;
        };
        let mut slot = self.state.slot_for(&ctx);
        let result = cmd.handle(m, &ctx, &mut slot).await;
        DispatchOutcome::Replied(result.replies)
    }

    /// Like `send`, but returns just the text of each reply. Empty vec
    /// when the command did not match, was denied, or produced no replies.
    pub async fn say(&self, cmd: &dyn Command, text: &str) -> Vec<String> {
        self.send(cmd, text).await.texts()
    }

    /// Drive a [`Monitor`] with the given text, using the tester's configured
    /// author and channel. Mirrors the `MonitorCommand` wrapper: trims the
    /// text and skips if empty. Monitors produce no replies, so this just
    /// triggers the side-effect (e.g. writing to the database).
    pub async fn observe(&self, monitor: &dyn Monitor, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let ctx = self.context();
        monitor.observe(&ctx, text).await;
    }

    /// Access the shared state map. Useful for tests that need to inspect
    /// scratch state across multiple `send` calls in the same context.
    pub fn state(&self) -> &StateMap {
        &self.state
    }
}

/// Outcome of [`CommandTester::send`]. Distinguishes the three reasons a
/// command might produce zero replies:
///
/// * `Denied` — `CommandMeta::permits` rejected (require_direct / caps).
/// * `NoMatch` — `Command::matches` returned `None`.
/// * `Replied(rs)` — the command ran; `rs` may still be empty (silent handler).
#[derive(Debug)]
pub enum DispatchOutcome {
    Replied(Vec<Reply>),
    Denied,
    NoMatch,
}

impl DispatchOutcome {
    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Denied)
    }
    pub fn is_no_match(&self) -> bool {
        matches!(self, Self::NoMatch)
    }
    pub fn replies(&self) -> Option<&Vec<Reply>> {
        match self {
            Self::Replied(rs) => Some(rs),
            _ => None,
        }
    }
    /// Plucks the texts from `Replied(_)`. Empty Vec otherwise.
    pub fn texts(&self) -> Vec<String> {
        match self {
            Self::Replied(rs) => rs.iter().map(|r| r.text.clone()).collect(),
            _ => Vec::new(),
        }
    }
    /// Panics unless this is `Replied(_)`. Convenient for tests that
    /// expect a reply and want to fail loudly otherwise.
    #[track_caller]
    pub fn unwrap_replies(self) -> Vec<Reply> {
        match self {
            Self::Replied(rs) => rs,
            other => panic!("expected Replied, got {other:?}"),
        }
    }
}
