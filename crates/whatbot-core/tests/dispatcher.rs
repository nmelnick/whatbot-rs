//! End-to-end tests for the `Dispatcher`. Pumps `RawEvent`s through a
//! dispatcher wired with mock identity resolution, mock commands, and a
//! captured outbound channel.
//!
//! Most tests use [`BotHarness`] for deterministic synchronization. The two
//! that construct a raw `Dispatcher` directly (`reply_routed_by_destination_service`
//! and `transcript_records_inbound_and_outbound`) need access to internals that
//! `BotHarness` does not expose.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::{mpsc, Mutex};

use whatbot_core::dispatcher::{DispatchError, IdentityResolver};
use whatbot_core::{
    Account, BotHarness, ChannelId, Command, CommandMeta, CommandResult, Context, Dispatcher,
    Event, MatchData, Priority, RawEvent, Registry, Reply, ServiceId, StateSlot, TranscriptEntry,
    TranscriptHandle, Visibility,
};

// ── Mocks ──────────────────────────────────────────────────────────────────

/// IdentityResolver that synthesizes an Account for any handle.
struct MockResolver;

#[async_trait]
impl IdentityResolver for MockResolver {
    async fn resolve(
        &self,
        service: &ServiceId,
        handle: &str,
        display: &str,
    ) -> Result<Account, DispatchError> {
        let mut a = Account::synthetic(service.clone(), handle);
        a.display = display.to_string();
        a.id = 1;
        Ok(a)
    }
}

/// A configurable command: matches every message, optionally produces a
/// reply, optionally sets stop/consumed, and counts handle() invocations.
struct FixedCommand {
    meta: CommandMeta,
    reply_text: Option<&'static str>,
    stop: bool,
    consumed: bool,
    handle_count: Arc<AtomicUsize>,
}

impl FixedCommand {
    fn new(
        name: &'static str,
        priority: Priority,
        reply_text: Option<&'static str>,
        stop: bool,
    ) -> (Arc<Self>, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let cmd = Arc::new(Self {
            meta: CommandMeta::new(name, priority, ""),
            reply_text,
            stop,
            consumed: false,
            handle_count: counter.clone(),
        });
        (cmd, counter)
    }

    /// Variant: emits no reply but marks the result as consumed (Karma's
    /// silent-apply shape). Used to test the cross-tier short-circuit.
    fn consuming(name: &'static str, priority: Priority) -> (Arc<Self>, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let cmd = Arc::new(Self {
            meta: CommandMeta::new(name, priority, ""),
            reply_text: None,
            stop: false,
            consumed: true,
            handle_count: counter.clone(),
        });
        (cmd, counter)
    }
}

#[async_trait]
impl Command for FixedCommand {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }
    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        if m.text.is_empty() {
            return None;
        }
        Some(MatchData::new(()))
    }
    async fn handle(&self, _m: MatchData, ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        self.handle_count.fetch_add(1, Ordering::SeqCst);
        let result = match self.reply_text {
            Some(text) => CommandResult::reply(ctx.reply_here(text)),
            None => CommandResult::empty(),
        };
        result.with_stop(self.stop).with_consumed(self.consumed)
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn raw_message(text: &str) -> RawEvent {
    RawEvent::Message {
        service: ServiceId::new("test"),
        channel: ChannelId::new("main"),
        visibility: Visibility::Public,
        author_handle: "nichelle".into(),
        author_display: "nichelle".into(),
        bot_handle: "whatbot".into(),
        bot_display: "whatbot".into(),
        text: text.into(),
        ts: Utc::now(),
        addressed_to_bot: true,
        provider_message_id: None,
    }
}

/// Spawn a dispatcher and return:
///   * the inbound sender (so the test can push RawEvents in)
///   * a Mutex<Vec<Reply>> capturing every routed Reply
///   * the JoinHandle of the dispatcher task
fn spawn(
    registry: Registry,
    transcript: Option<TranscriptHandle>,
) -> (
    mpsc::Sender<RawEvent>,
    Arc<Mutex<Vec<Reply>>>,
    tokio::task::JoinHandle<()>,
) {
    let mut dispatcher = Dispatcher::new(registry, Arc::new(MockResolver), 16);
    if let Some(t) = transcript {
        dispatcher.set_transcript(t);
    }
    let inbound = dispatcher.inbound_sender();
    let captured: Arc<Mutex<Vec<Reply>>> = Arc::new(Mutex::new(Vec::new()));
    let (tx, mut rx) = mpsc::channel::<Reply>(16);
    dispatcher.register_outbound(ServiceId::new("test"), tx);

    let cap = captured.clone();
    tokio::spawn(async move {
        while let Some(r) = rx.recv().await {
            cap.lock().await.push(r);
        }
    });

    let task = tokio::spawn(async move {
        dispatcher.run().await.expect("dispatcher run");
    });

    (inbound, captured, task)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn primary_output_skips_extension_and_last() {
    let (primary, _) = FixedCommand::new("p", Priority::Primary, Some("from primary"), false);
    let (extension, _) = FixedCommand::new("e", Priority::Extension, Some("from extension"), false);
    let (last, _) = FixedCommand::new("l", Priority::Last, Some("from last"), false);

    let bot = BotHarness::builder()
        .install_arc(primary)
        .install_arc(extension)
        .install_arc(last)
        .build()
        .await;

    let replies = bot.say("nichelle", "anything").await;
    assert_eq!(replies, vec!["from primary".to_string()]);
}

#[tokio::test]
async fn last_runs_when_nothing_earlier_produced() {
    let (core_silent, _) = FixedCommand::new("silent", Priority::Core, None, false);
    let (last, _) = FixedCommand::new("l", Priority::Last, Some("from last"), false);

    let bot = BotHarness::builder()
        .install_arc(core_silent)
        .install_arc(last)
        .build()
        .await;

    let replies = bot.say("nichelle", "ping").await;
    assert_eq!(replies, vec!["from last".to_string()]);
}

#[tokio::test]
async fn stop_short_circuits_within_a_tier() {
    let (first, _) = FixedCommand::new("first", Priority::Core, Some("first reply"), true);
    let (second, _) = FixedCommand::new("second", Priority::Core, Some("second reply"), false);

    let bot = BotHarness::builder()
        .install_arc(first)
        .install_arc(second)
        .build()
        .await;

    let replies = bot.say("nichelle", "ping").await;
    assert_eq!(replies, vec!["first reply".to_string()]);
}

#[tokio::test]
async fn require_direct_blocks_undirected_messages() {
    let strict = Arc::new(FixedCommand {
        meta: CommandMeta::core("strict", "").require_direct(),
        reply_text: Some("ack"),
        stop: false,
        consumed: false,
        handle_count: Arc::new(AtomicUsize::new(0)),
    });

    let bot = BotHarness::builder().install_arc(strict).build().await;
    let replies = bot.say_unaddressed("nichelle", "hi").await;
    assert!(
        replies.is_empty(),
        "require_direct must suppress command when not addressed"
    );
}

#[tokio::test]
async fn identity_is_resolved_into_context() {
    // Encode resolved context fields as reply text so assertions stay in
    // the test body rather than inside the spawned dispatcher task (where
    // panics would not surface as test failures).
    struct ReportIdentity;
    #[async_trait]
    impl Command for ReportIdentity {
        fn meta(&self) -> &CommandMeta {
            static META: std::sync::OnceLock<CommandMeta> = std::sync::OnceLock::new();
            META.get_or_init(|| CommandMeta::core("report", ""))
        }
        fn matches(&self, e: &Event, _c: &Context) -> Option<MatchData> {
            let Event::Message(m) = e else { return None };
            if m.text.is_empty() {
                return None;
            }
            Some(MatchData::new(()))
        }
        async fn handle(&self, _m: MatchData, ctx: &Context, _s: &mut StateSlot) -> CommandResult {
            CommandResult::reply(ctx.reply_here(format!(
                "author={} display={} bot={}",
                ctx.author.handle, ctx.author.display, ctx.bot.handle
            )))
        }
    }

    let bot = BotHarness::builder().install(ReportIdentity).build().await;
    let replies = bot.say("nichelle", "hi").await;
    assert_eq!(
        replies,
        vec!["author=nichelle display=nichelle bot=whatbot".to_string()]
    );
}

#[tokio::test]
async fn reply_routed_by_destination_service() {
    // Register two outbound channels; verify a reply to "test" arrives on
    // the test channel and not the other.
    let (cmd, _) = FixedCommand::new("c", Priority::Core, Some("hi"), false);
    let mut reg = Registry::new();
    reg.install(cmd);

    let mut dispatcher = Dispatcher::new(reg, Arc::new(MockResolver), 16);
    let inbound = dispatcher.inbound_sender();
    let (test_tx, mut test_rx) = mpsc::channel::<Reply>(8);
    let (other_tx, mut other_rx) = mpsc::channel::<Reply>(8);
    dispatcher.register_outbound(ServiceId::new("test"), test_tx);
    dispatcher.register_outbound(ServiceId::new("other"), other_tx);

    let task = tokio::spawn(async move {
        dispatcher.run().await.expect("dispatcher run");
    });

    inbound.send(raw_message("ping")).await.unwrap();
    drop(inbound);
    let _ = task.await;

    let mut test_count = 0usize;
    while test_rx.try_recv().is_ok() {
        test_count += 1;
    }
    let mut other_count = 0usize;
    while other_rx.try_recv().is_ok() {
        other_count += 1;
    }
    assert_eq!(test_count, 1);
    assert_eq!(other_count, 0);
}

#[tokio::test]
async fn transcript_records_inbound_and_outbound() {
    let (handle, mut rx) = TranscriptHandle::channel(16);
    let (cmd, _) = FixedCommand::new("c", Priority::Core, Some("the reply"), false);

    let mut reg = Registry::new();
    reg.install(cmd);

    let (inbound, _captured, task) = spawn(reg, Some(handle));
    inbound.send(raw_message("the input")).await.unwrap();
    drop(inbound);
    let _ = task.await;

    let mut entries: Vec<TranscriptEntry> = Vec::new();
    while let Ok(e) = rx.try_recv() {
        entries.push(e);
    }
    assert_eq!(entries.len(), 2, "one inbound + one outbound entry");
    assert!(
        matches!(entries[0].direction, whatbot_core::Direction::Incoming),
        "first entry must be Incoming, got {:?}",
        entries[0].direction
    );
    assert_eq!(entries[0].text, "the input");
    assert_eq!(entries[0].speaker, "nichelle");
    assert!(matches!(
        entries[1].direction,
        whatbot_core::Direction::Outgoing
    ));
    assert_eq!(entries[1].text, "the reply");
    assert_eq!(entries[1].speaker, "whatbot");
}

#[tokio::test]
async fn consumed_silently_skips_lower_tiers() {
    // A Core command that emits no reply but marks `consumed = true`
    // (Karma's silent ++/-- shape) must skip Extension and Last just as
    // if it had replied. Without this rule, an enabled command at `Last`
    // would speak even when Karma swallowed the message.
    let (core_consumer, _) = FixedCommand::consuming("karma-like", Priority::Core);
    let (last_loud, _) = FixedCommand::new("would-parrot", Priority::Last, Some("PARROT"), false);

    let bot = BotHarness::builder()
        .install_arc(core_consumer)
        .install_arc(last_loud)
        .build()
        .await;

    let replies = bot.say("nichelle", "rust++").await;
    assert!(
        replies.is_empty(),
        "Last must not fire when Core consumed silently"
    );
}

#[tokio::test]
async fn silent_unconsumed_still_runs_lower_tiers() {
    // Control: an empty result *without* consumed=true does NOT count as
    // handled, so Last still fires. This pins the difference.
    let (core_passive, _) = FixedCommand::new("noop", Priority::Core, None, false);
    let (last_loud, _) = FixedCommand::new("would-parrot", Priority::Last, Some("PARROT"), false);

    let bot = BotHarness::builder()
        .install_arc(core_passive)
        .install_arc(last_loud)
        .build()
        .await;

    let replies = bot.say("nichelle", "hello").await;
    assert_eq!(replies, vec!["PARROT".to_string()]);
}
