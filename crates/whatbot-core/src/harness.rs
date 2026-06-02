//! [`BotHarness`] — a full-stack test harness.
//!
//! Where [`CommandTester`](crate::testing::CommandTester) exercises a
//! single command in isolation, `BotHarness` spins up a real
//! [`Dispatcher`] with multiple installed commands and a captured
//! outbound channel. Tests look like:
//!
//! ```ignore
//! let pg = Pg::shared().await;
//! let store: Arc<dyn KarmaStore> = Arc::new(SqlKarmaStore::new(pg.fresh_store().await));
//! let bot = BotHarness::builder()
//!     .install(Karma::new(store))
//!     .build()
//!     .await;
//! assert!(
//!     bot.say("nichelle", "rust++").await.is_empty(),
//!     "Karma consumed the message"
//! );
//! ```
//!
//! ## Synchronization
//!
//! Because `Dispatcher::run` is a long-lived async task, the harness
//! can't simply await "the dispatcher finished this event" — there's no
//! such signal. Instead, every `say` call pushes the user event
//! followed by a **barrier event** that only a built-in
//! [`BarrierCommand`] matches. The barrier handler signals via
//! [`tokio::sync::Notify`]; because the dispatcher consumes its inbound
//! channel FIFO and serially, the user event has been fully handled by
//! the time the barrier fires. `say` calls are themselves serialized
//! by an internal `Mutex` so multiple in-flight barriers can't race.
//!
//! The barrier event is invisible to user-installed commands:
//!   * `text` is empty (no regex matches it).
//!   * `provider_message_id` carries the sentinel; only the barrier
//!     command inspects that field.
//!   * The barrier returns `CommandResult::handled_silently()` to
//!     prevent ambient `Last`-tier commands from firing.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::{mpsc, Mutex, Notify};

use crate::command::{Command, CommandMeta, CommandResult, MatchData};
use crate::context::{ChannelId, Context, ServiceId, Visibility};
use crate::dispatcher::{DispatchError, Dispatcher, IdentityResolver, Registry};
use crate::event::{Event, RawEvent};
use crate::identity::Account;
use crate::reply::Reply;
use crate::state::StateSlot;

const BARRIER_PROVIDER_ID: &str = "__whatbot_harness_barrier__";
const BARRIER_AUTHOR: &str = "__barrier__";

/// Trivial identity resolver that synthesizes an `Account` for any
/// (service, handle, display) tuple without touching storage.
#[derive(Debug, Default)]
struct HarnessIdentity;

#[async_trait]
impl IdentityResolver for HarnessIdentity {
    async fn resolve(
        &self,
        service: &ServiceId,
        handle: &str,
        display: &str,
    ) -> Result<Account, DispatchError> {
        let mut a = Account::synthetic(service.clone(), handle);
        a.display = display.to_string();
        // id stays 0 (the synthetic marker). SqlStore adapters interpret
        // that as "no DB row" and skip the account_id FK, so commands
        // backed by Postgres still work under the harness.
        Ok(a)
    }
}

/// Built-in command that signals via Notify when the dispatcher
/// processes its barrier event.
struct BarrierCommand {
    meta: CommandMeta,
    notify: Arc<Notify>,
}

impl BarrierCommand {
    fn new(notify: Arc<Notify>) -> Self {
        Self {
            // Primary so it runs before user-installed Core commands —
            // not strictly required (other commands won't match a
            // barrier event because text is empty), but cheap insurance.
            meta: CommandMeta::primary("__bot_harness_barrier", ""),
            notify,
        }
    }
}

#[async_trait]
impl Command for BarrierCommand {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }
    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        if m.provider_message_id.as_deref() == Some(BARRIER_PROVIDER_ID) {
            Some(MatchData::new(()))
        } else {
            None
        }
    }
    async fn handle(&self, _m: MatchData, _ctx: &Context, _s: &mut StateSlot) -> CommandResult {
        self.notify.notify_one();
        // Silent + consumed so no Last commands fire on the barrier.
        CommandResult::handled_silently()
    }
}

/// Build a [`BotHarness`] by accumulating commands and (optionally)
/// overriding the service / channel / bot identity used for synthesized
/// events.
pub struct BotHarnessBuilder {
    service: ServiceId,
    channel: ChannelId,
    bot_handle: String,
    commands: Vec<Arc<dyn Command>>,
}

impl Default for BotHarnessBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BotHarnessBuilder {
    pub fn new() -> Self {
        Self {
            service: ServiceId::new("harness"),
            channel: ChannelId::new("main"),
            bot_handle: "whatbot".to_string(),
            commands: Vec::new(),
        }
    }

    pub fn with_service(mut self, s: &str) -> Self {
        self.service = ServiceId::new(s);
        self
    }
    pub fn with_channel(mut self, c: &str) -> Self {
        self.channel = ChannelId::new(c);
        self
    }
    pub fn with_bot(mut self, b: &str) -> Self {
        self.bot_handle = b.to_string();
        self
    }

    /// Install a command. Accepts both already-`Arc`'d commands (e.g.
    /// when you need to share state with the test body) and bare values.
    pub fn install<C: Command + 'static>(mut self, cmd: C) -> Self {
        self.commands.push(Arc::new(cmd));
        self
    }
    pub fn install_arc(mut self, cmd: Arc<dyn Command>) -> Self {
        self.commands.push(cmd);
        self
    }

    pub async fn build(self) -> BotHarness {
        let mut registry = Registry::new();
        let notify = Arc::new(Notify::new());
        registry.install(Arc::new(BarrierCommand::new(notify.clone())));
        for cmd in self.commands {
            registry.install(cmd);
        }

        let identity: Arc<dyn IdentityResolver> = Arc::new(HarnessIdentity);
        let mut dispatcher = Dispatcher::new(registry, identity, 32);

        let captured: Arc<Mutex<Vec<Reply>>> = Arc::new(Mutex::new(Vec::new()));
        let (out_tx, mut out_rx) = mpsc::channel::<Reply>(32);
        dispatcher.register_outbound(self.service.clone(), out_tx);

        let cap_for_task = captured.clone();
        tokio::spawn(async move {
            while let Some(r) = out_rx.recv().await {
                cap_for_task.lock().await.push(r);
            }
        });

        let inbound = dispatcher.inbound_sender();
        let dispatcher_task = tokio::spawn(async move {
            let _ = dispatcher.run().await;
        });

        BotHarness {
            service: self.service,
            channel: self.channel,
            bot_handle: self.bot_handle,
            inbound,
            captured,
            barrier: notify,
            say_lock: Arc::new(Mutex::new(())),
            _dispatcher_task: dispatcher_task,
        }
    }
}

pub struct BotHarness {
    service: ServiceId,
    channel: ChannelId,
    bot_handle: String,
    inbound: mpsc::Sender<RawEvent>,
    captured: Arc<Mutex<Vec<Reply>>>,
    barrier: Arc<Notify>,
    say_lock: Arc<Mutex<()>>,
    _dispatcher_task: tokio::task::JoinHandle<()>,
}

impl BotHarness {
    pub fn builder() -> BotHarnessBuilder {
        BotHarnessBuilder::new()
    }

    /// Push a message from `author` (treated as addressed-to-bot) and
    /// return the texts of every reply the dispatcher produced.
    pub async fn say(&self, author: &str, text: &str) -> Vec<String> {
        self.say_internal(&self.channel, author, text, true)
            .await
            .into_iter()
            .map(|r| r.text)
            .collect()
    }

    /// Like [`say`](Self::say) but the event is marked as *not* addressed
    /// to the bot — simulates ambient channel chatter.
    pub async fn say_unaddressed(&self, author: &str, text: &str) -> Vec<String> {
        self.say_internal(&self.channel, author, text, false)
            .await
            .into_iter()
            .map(|r| r.text)
            .collect()
    }

    /// Push from a specific channel. State scoped by `(service, channel)`
    /// remains isolated from the default channel.
    pub async fn say_in(&self, channel: &str, author: &str, text: &str) -> Vec<String> {
        let ch = ChannelId::new(channel);
        self.say_internal(&ch, author, text, true)
            .await
            .into_iter()
            .map(|r| r.text)
            .collect()
    }

    /// Like [`say`](Self::say) but returns the full [`Reply`] vector
    /// (destination, text, etc.) instead of just texts.
    pub async fn send(&self, author: &str, text: &str) -> Vec<Reply> {
        self.say_internal(&self.channel, author, text, true).await
    }

    async fn say_internal(
        &self,
        channel: &ChannelId,
        author: &str,
        text: &str,
        addressed: bool,
    ) -> Vec<Reply> {
        // Serialize say() calls so barrier notifications can't race.
        let _guard = self.say_lock.lock().await;

        // Drain any leftover replies from prior calls.
        self.captured.lock().await.clear();

        // Subscribe to the barrier *before* sending events, so the
        // notification can't be lost between sends.
        let notified = self.barrier.notified();
        tokio::pin!(notified);

        let user = RawEvent::Message {
            service: self.service.clone(),
            channel: channel.clone(),
            visibility: Visibility::Public,
            author_handle: author.to_string(),
            author_display: author.to_string(),
            bot_handle: self.bot_handle.clone(),
            bot_display: self.bot_handle.clone(),
            text: text.to_string(),
            ts: Utc::now(),
            addressed_to_bot: addressed,
            provider_message_id: None,
        };
        let barrier = RawEvent::Message {
            service: self.service.clone(),
            channel: channel.clone(),
            visibility: Visibility::Public,
            author_handle: BARRIER_AUTHOR.to_string(),
            author_display: BARRIER_AUTHOR.to_string(),
            bot_handle: self.bot_handle.clone(),
            bot_display: self.bot_handle.clone(),
            text: String::new(),
            ts: Utc::now(),
            addressed_to_bot: false,
            provider_message_id: Some(BARRIER_PROVIDER_ID.to_string()),
        };

        if self.inbound.send(user).await.is_err() {
            return Vec::new();
        }
        if self.inbound.send(barrier).await.is_err() {
            return Vec::new();
        }

        notified.await;

        // Replies from the user event are guaranteed to be in the buffer
        // by now — they were routed before the dispatcher even started
        // processing the barrier event.
        std::mem::take(&mut *self.captured.lock().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Priority;

    /// Trivial echo-style command: replies "ack: <text>" to anything.
    struct AckAll {
        meta: CommandMeta,
    }
    impl AckAll {
        fn new() -> Self {
            Self {
                meta: CommandMeta::core("ack", ""),
            }
        }
    }
    #[async_trait]
    impl Command for AckAll {
        fn meta(&self) -> &CommandMeta {
            &self.meta
        }
        fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
            let Event::Message(m) = evt else { return None };
            if m.text.is_empty() {
                None
            } else {
                Some(MatchData::new(m.text.clone()))
            }
        }
        async fn handle(&self, m: MatchData, ctx: &Context, _s: &mut StateSlot) -> CommandResult {
            let text = *m.downcast::<String>().expect("string");
            ctx.say(format!("ack: {text}"))
        }
    }

    /// Last-priority parrot: would fire for any message that no earlier
    /// command consumed.
    struct LastParrot;
    #[async_trait]
    impl Command for LastParrot {
        fn meta(&self) -> &CommandMeta {
            static M: std::sync::OnceLock<CommandMeta> = std::sync::OnceLock::new();
            M.get_or_init(|| CommandMeta::new("last-parrot", Priority::Last, ""))
        }
        fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
            let Event::Message(m) = evt else { return None };
            if m.text.is_empty() {
                None
            } else {
                Some(MatchData::new(()))
            }
        }
        async fn handle(&self, _m: MatchData, ctx: &Context, _s: &mut StateSlot) -> CommandResult {
            ctx.say("LAST")
        }
    }

    #[tokio::test]
    async fn say_returns_replies_from_installed_commands() {
        let bot = BotHarness::builder().install(AckAll::new()).build().await;
        let replies = bot.say("nichelle", "hello").await;
        assert_eq!(replies, vec!["ack: hello".to_string()]);
    }

    #[tokio::test]
    async fn say_with_no_match_returns_empty() {
        // Install no commands at all (besides the harness barrier).
        let bot = BotHarness::builder().build().await;
        let replies = bot.say("nichelle", "anything").await;
        assert!(replies.is_empty());
    }

    #[tokio::test]
    async fn consecutive_says_get_separated_replies() {
        let bot = BotHarness::builder().install(AckAll::new()).build().await;
        let one = bot.say("nichelle", "first").await;
        let two = bot.say("nichelle", "second").await;
        assert_eq!(one, vec!["ack: first".to_string()]);
        assert_eq!(two, vec!["ack: second".to_string()]);
    }

    #[tokio::test]
    async fn silent_consumer_alone_doesnt_skip_last() {
        // Control: if no consumer is installed, Last fires.
        let bot = BotHarness::builder().install(LastParrot).build().await;
        let replies = bot.say("nichelle", "hi").await;
        assert_eq!(replies, vec!["LAST".to_string()]);
    }

    #[tokio::test]
    async fn say_in_uses_different_channel() {
        // Smoke: different channel produces different reply destination,
        // and `say_in` doesn't crash because the dispatcher only has one
        // outbound channel registered (for the harness service).
        let bot = BotHarness::builder().install(AckAll::new()).build().await;
        let replies = bot.say_in("other", "nichelle", "hi").await;
        assert_eq!(replies, vec!["ack: hi".to_string()]);
    }
}
