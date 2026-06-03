//! A lightweight passive message listener.
//!
//! A `Monitor` is a [`Command`] that runs on every non-empty message at
//! `Priority::Primary` and returns no replies.

use async_trait::async_trait;

use crate::command::{Command, CommandMeta, CommandResult, MatchData};
use crate::context::Context;
use crate::event::Event;
use crate::state::StateSlot;

/// Observe every non-empty message without producing a reply.
#[async_trait]
pub trait Monitor: Send + Sync {
    fn name(&self) -> &'static str;
    async fn observe(&self, ctx: &Context, text: &str);
}

struct Captured(String);

pub(crate) struct MonitorCommand<M> {
    meta: CommandMeta,
    inner: M,
}

impl<M: Monitor> MonitorCommand<M> {
    pub(crate) fn new(inner: M) -> Self {
        Self {
            meta: CommandMeta::primary(inner.name(), ""),
            inner,
        }
    }
}

#[async_trait]
impl<M: Monitor + 'static> Command for MonitorCommand<M> {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else {
            return None;
        };
        let text = m.text.trim();
        if text.is_empty() {
            return None;
        }
        Some(MatchData::new(Captured(text.to_string())))
    }

    async fn handle(&self, m: MatchData, ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        let Captured(text) = match m.downcast::<Captured>() {
            Ok(c) => *c,
            Err(_) => return CommandResult::empty(),
        };
        self.inner.observe(ctx, &text).await;
        CommandResult::empty()
    }
}
