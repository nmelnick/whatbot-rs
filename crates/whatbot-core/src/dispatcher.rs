use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::command::{Command, Priority};
use crate::context::{ChannelId, Context, ServiceId, Visibility};
use crate::event::{Event, RawEvent};
use crate::identity::Account;
use crate::mentions::{default_mention_renderer, MentionRenderer};
use crate::message::Message;
use crate::monitor::{Monitor, MonitorCommand};
use crate::reply::{Destination, Reply};
use crate::state::StateMap;
use crate::transcript::{Direction, TranscriptEntry, TranscriptHandle};

/// Registry of installed commands, grouped by priority tier.
#[derive(Default)]
pub struct Registry {
    by_priority: HashMap<Priority, Vec<Arc<dyn Command>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(&mut self, cmd: Arc<dyn Command>) {
        let p = cmd.meta().priority;
        self.by_priority.entry(p).or_default().push(cmd);
    }

    pub fn install_monitor<M: Monitor + 'static>(&mut self, monitor: M) {
        self.install(Arc::new(MonitorCommand::new(monitor)));
    }

    pub fn commands_at(&self, priority: Priority) -> &[Arc<dyn Command>] {
        self.by_priority
            .get(&priority)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Iterate over every installed command, in priority order
    pub fn iter_commands(&self) -> impl Iterator<Item = &Arc<dyn Command>> {
        Priority::ALL
            .iter()
            .flat_map(move |p| self.commands_at(*p).iter())
    }
}

/// Resolves a raw service-native handle into a persisted [`Account`]
#[async_trait]
pub trait IdentityResolver: Send + Sync {
    async fn resolve(
        &self,
        service: &ServiceId,
        handle: &str,
        display: &str,
    ) -> Result<Account, DispatchError>;
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("identity resolution failed: {0}")]
    Identity(String),
    #[error("send failed: {0}")]
    Send(String),
}

pub struct Dispatcher {
    registry: Arc<Registry>,
    state: StateMap,
    identity: Arc<dyn IdentityResolver>,
    inbound_rx: mpsc::Receiver<RawEvent>,
    inbound_tx: mpsc::Sender<RawEvent>,
    outbound: HashMap<ServiceId, mpsc::Sender<Reply>>,
    renderers: HashMap<ServiceId, Arc<dyn MentionRenderer>>,
    transcript: Option<TranscriptHandle>,
}

impl Dispatcher {
    pub fn new(
        registry: Registry,
        identity: Arc<dyn IdentityResolver>,
        channel_capacity: usize,
    ) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(channel_capacity);
        Self {
            registry: Arc::new(registry),
            state: StateMap::new(),
            identity,
            inbound_rx,
            inbound_tx,
            outbound: HashMap::new(),
            renderers: HashMap::new(),
            transcript: None,
        }
    }

    /// Register a service-specific mention renderer
    pub fn register_mention_renderer(
        &mut self,
        service: ServiceId,
        renderer: Arc<dyn MentionRenderer>,
    ) {
        self.renderers.insert(service, renderer);
    }

    /// Attach a transcript sink
    pub fn set_transcript(&mut self, handle: TranscriptHandle) {
        self.transcript = Some(handle);
    }

    /// Get a clone of the inbound sender for each IO
    pub fn inbound_sender(&self) -> mpsc::Sender<RawEvent> {
        self.inbound_tx.clone()
    }

    /// Register an outbound channel for a given service
    pub fn register_outbound(&mut self, service: ServiceId, sender: mpsc::Sender<Reply>) {
        self.outbound.insert(service, sender);
    }

    /// do-it.pl
    pub async fn run(self) -> Result<(), DispatchError> {
        let Self {
            registry,
            state,
            identity,
            mut inbound_rx,
            inbound_tx,
            outbound,
            renderers,
            transcript,
        } = self;
        drop(inbound_tx);
        let dispatch = DispatchCore {
            registry,
            state,
            identity,
            outbound,
            renderers,
            transcript,
        };
        while let Some(raw) = inbound_rx.recv().await {
            if let Err(err) = dispatch.handle_raw(raw).await {
                error!(?err, "dispatch error");
            }
        }
        Ok(())
    }
}

struct DispatchCore {
    registry: Arc<Registry>,
    state: StateMap,
    identity: Arc<dyn IdentityResolver>,
    outbound: HashMap<ServiceId, mpsc::Sender<Reply>>,
    renderers: HashMap<ServiceId, Arc<dyn MentionRenderer>>,
    transcript: Option<TranscriptHandle>,
}

impl DispatchCore {
    fn renderer_for(&self, service: &ServiceId) -> Arc<dyn MentionRenderer> {
        self.renderers
            .get(service)
            .cloned()
            .unwrap_or_else(default_mention_renderer)
    }
}

impl DispatchCore {
    async fn handle_raw(&self, raw: RawEvent) -> Result<(), DispatchError> {
        let event = self.resolve(raw).await?;
        let Some(ctx) = event.context().cloned() else {
            return Ok(());
        };

        if let (Some(t), Event::Message(m)) = (&self.transcript, &event) {
            t.record(TranscriptEntry {
                ts: m.ts,
                direction: Direction::Incoming,
                service: ctx.service.clone(),
                channel: ctx.channel.clone(),
                visibility: ctx.visibility.clone(),
                speaker: ctx.author.display.clone(),
                text: m.text.clone(),
            });
        }

        let mut produced_so_far = false;
        for priority in Priority::ALL {
            if produced_so_far && matches!(priority, Priority::Extension | Priority::Last) {
                debug!(?priority, "skipping due to earlier output");
                break;
            }
            for cmd in self.registry.commands_at(priority) {
                if !cmd.meta().permits(&ctx) {
                    continue;
                }
                let Some(m) = cmd.matches(&event, &ctx) else {
                    continue;
                };
                let mut slot = self.state.slot_for(&ctx);
                let result = cmd.handle(m, &ctx, &mut slot).await;
                if !result.replies.is_empty() || result.consumed {
                    produced_so_far = true;
                }
                for reply in result.replies {
                    if let Err(e) = self.send_reply(reply, &ctx).await {
                        warn!(?e, "reply send failed");
                    }
                }
                if result.stop {
                    break;
                }
            }
        }
        Ok(())
    }

    async fn send_reply(&self, reply: Reply, ctx: &Context) -> Result<(), DispatchError> {
        if let Some(t) = &self.transcript {
            let (channel, visibility) = match &reply.destination {
                Destination::Channel { channel, .. } | Destination::ReplyTo { channel, .. } => {
                    let vis = if channel == &ctx.channel {
                        ctx.visibility.clone()
                    } else {
                        crate::context::Visibility::Public
                    };
                    (channel.clone(), vis)
                }
                Destination::Direct { account, .. } => (
                    crate::context::ChannelId::new(format!("dm-{}", account.handle)),
                    crate::context::Visibility::Private,
                ),
            };
            t.record(TranscriptEntry {
                ts: chrono::Utc::now(),
                direction: Direction::Outgoing,
                service: reply.destination.service().clone(),
                channel,
                visibility,
                speaker: ctx.bot.display.clone(),
                text: reply.text.clone(),
            });
        }

        let svc = reply.destination.service().clone();
        let Some(tx) = self.outbound.get(&svc) else {
            return Err(DispatchError::Send(format!(
                "no outbound channel for service {}",
                svc.as_str()
            )));
        };
        tx.send(reply)
            .await
            .map_err(|e| DispatchError::Send(e.to_string()))
    }

    async fn resolve(&self, raw: RawEvent) -> Result<Event, DispatchError> {
        match raw {
            RawEvent::Message {
                service,
                channel,
                visibility,
                author_handle,
                author_display,
                bot_handle,
                bot_display,
                text,
                ts,
                addressed_to_bot,
                provider_message_id,
            } => {
                let author = self
                    .identity
                    .resolve(&service, &author_handle, &author_display)
                    .await?;
                let bot = self
                    .identity
                    .resolve(&service, &bot_handle, &bot_display)
                    .await?;
                let mention_renderer = self.renderer_for(&service);
                let ctx = Context {
                    service,
                    channel,
                    visibility,
                    author,
                    bot,
                    addressed_to_bot,
                    mention_renderer,
                };
                let msg = Message {
                    ctx,
                    text,
                    mentions: Vec::new(),
                    links: Vec::new(),
                    media: Vec::new(),
                    ts,
                    provider_message_id,
                };
                Ok(Event::Message(msg))
            }
            RawEvent::Joined {
                service,
                channel,
                who_handle,
                who_display,
            } => {
                let who = self
                    .identity
                    .resolve(&service, &who_handle, &who_display)
                    .await?;
                let ctx = self.synthetic_ctx(service, channel, who.clone());
                Ok(Event::Joined { ctx, who })
            }
            RawEvent::Left {
                service,
                channel,
                who_handle,
                who_display,
            } => {
                let who = self
                    .identity
                    .resolve(&service, &who_handle, &who_display)
                    .await?;
                let ctx = self.synthetic_ctx(service, channel, who.clone());
                Ok(Event::Left { ctx, who })
            }
            RawEvent::Topic {
                service,
                channel,
                topic,
                by_handle,
                by_display,
            } => {
                let by = self
                    .identity
                    .resolve(&service, &by_handle, &by_display)
                    .await?;
                let ctx = self.synthetic_ctx(service, channel, by.clone());
                Ok(Event::Topic { ctx, topic, by })
            }
        }
    }

    fn synthetic_ctx(&self, service: ServiceId, channel: ChannelId, who: Account) -> Context {
        let mention_renderer = self.renderer_for(&service);
        Context {
            service: service.clone(),
            channel,
            visibility: Visibility::Public,
            author: who.clone(),
            bot: Account::synthetic(service, ""),
            addressed_to_bot: false,
            mention_renderer,
        }
    }
}
