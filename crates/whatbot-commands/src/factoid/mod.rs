//! Factoid: the "x is y" memory

pub mod adapter;
pub mod store;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;

use whatbot_core::{
    match_data, Command, CommandMeta, CommandResult, Context, Event, MatchData, StateSlot,
};

pub use adapter::SqlFactoidStore;
pub use store::{FactoidStore, FactoidStoreError};

static RE_WHAT_IS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(?:wtf|what|who)\s+(?:is|are)\s+(.+?)\s*\??$").unwrap());
static RE_ASSIGN: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^(.+?)\s+(is|are)\s+(.+)$").unwrap());
static RE_FORGET: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^forget\s+(.+)$").unwrap());
static RE_WHO_SAID: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^who\s+said\s+that\??$").unwrap());
static RE_WHEN_WAS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^when\s+was\s+that\??$").unwrap());

#[derive(Default, Debug, Clone)]
pub struct FactoidScratch {
    pub who_said: Option<String>,
    pub when_was: Option<DateTime<Utc>>,
}

enum Action {
    WhatIs(String),
    Assign {
        subject: String,
        is_plural: bool,
        description: String,
    },
    Forget(String),
    WhoSaid,
    WhenWas,
}

pub struct Factoid {
    meta: CommandMeta,
    store: Arc<dyn FactoidStore>,
}

impl Factoid {
    pub fn new(store: Arc<dyn FactoidStore>) -> Self {
        Self {
            meta: CommandMeta::core(
                "factoid",
                "x is y; what is x; forget x; who said that; when was that",
            ),
            store,
        }
    }
}

#[async_trait]
impl Command for Factoid {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        let text = m.text.trim();

        if RE_WHO_SAID.is_match(text) {
            return Some(MatchData::new(Action::WhoSaid));
        }
        if RE_WHEN_WAS.is_match(text) {
            return Some(MatchData::new(Action::WhenWas));
        }
        if let Some(caps) = RE_FORGET.captures(text) {
            return Some(MatchData::new(Action::Forget(caps[1].trim().to_string())));
        }
        if let Some(caps) = RE_WHAT_IS.captures(text) {
            return Some(MatchData::new(Action::WhatIs(caps[1].trim().to_string())));
        }
        if let Some(caps) = RE_ASSIGN.captures(text) {
            let mut subject = caps[1].trim().to_string();
            // Don't treat questions as assignments.
            if matches!(
                subject.to_lowercase().as_str(),
                "wtf" | "what" | "who" | "when" | "where" | "why"
            ) {
                return None;
            }
            if ctx.addressed_to_bot && subject.eq_ignore_ascii_case("you") {
                subject = ctx.bot.display.clone();
            }
            let is_plural = caps[2].eq_ignore_ascii_case("are");
            let description = caps[3].trim().trim_end_matches('.').trim().to_string();
            if description.is_empty() {
                return None;
            }
            return Some(MatchData::new(Action::Assign {
                subject,
                is_plural,
                description,
            }));
        }
        None
    }

    async fn handle(&self, m: MatchData, ctx: &Context, state: &mut StateSlot) -> CommandResult {
        let action = match_data!(m => Action);
        match action {
            Action::WhatIs(subject) => self.retrieve(&subject, ctx, state).await,
            Action::Assign {
                subject,
                is_plural,
                description,
            } => {
                self.assign(&subject, is_plural, &description, ctx, state)
                    .await
            }
            Action::Forget(subject) => self.forget(&subject, ctx).await,
            Action::WhoSaid => self.who_said(ctx, state).await,
            Action::WhenWas => self.when_was(ctx, state).await,
        }
    }
}

impl Factoid {
    async fn assign(
        &self,
        subject: &str,
        is_plural: bool,
        description: &str,
        ctx: &Context,
        _state: &mut StateSlot,
    ) -> CommandResult {
        let id = match self.store.ensure(subject, is_plural).await {
            Ok(id) => id,
            Err(e) => return error_reply(ctx, "failed to record that", &e),
        };
        // Split on " or " to allow multi-fact assignment, mirroring old whatbot.
        for fact in description.split(" or ") {
            let fact = fact.trim();
            if fact.is_empty() {
                continue;
            }
            if let Err(e) = self.store.add_fact(id, fact, Some(&ctx.author)).await {
                return error_reply(ctx, "failed to record that", &e);
            }
        }
        // Respond when the user addressed us explicitly, otherwise, silently gather info
        if ctx.addressed_to_bot {
            ctx.say(format!("OK, {}.", ctx.author.display))
        } else {
            CommandResult::empty()
        }
    }

    async fn retrieve(&self, subject: &str, ctx: &Context, state: &mut StateSlot) -> CommandResult {
        // Direct retrieval: unknown subjects produce a "no idea" reply, and
        // silent-flagged factoids speak anyway because the user asked.
        retrieve_shared(&*self.store, subject, ctx, state, true).await
    }

    async fn forget(&self, subject: &str, ctx: &Context) -> CommandResult {
        match self.store.forget(subject).await {
            Ok(true) => ctx
                .say(format!("I forgot \"{subject}\", {}.", ctx.author.display))
                .with_stop(true),
            _ => CommandResult::empty(),
        }
    }

    async fn who_said(&self, ctx: &Context, state: &mut StateSlot) -> CommandResult {
        let said = state
            .with::<FactoidScratch, _, _>(|s| s.who_said.clone())
            .await;
        match said {
            Some(name) if name == ctx.author.handle => ctx
                .say(format!("{}: it was YOU!", ctx.author.display))
                .with_stop(true),
            Some(name) => ctx
                .say(format!("{}: {}", ctx.author.display, name))
                .with_stop(true),
            None => CommandResult::empty(),
        }
    }

    async fn when_was(&self, ctx: &Context, state: &mut StateSlot) -> CommandResult {
        let when = state.with::<FactoidScratch, _, _>(|s| s.when_was).await;
        match when {
            Some(t) => ctx
                .say(format!(
                    "Looks like it was on {}.",
                    t.format("%Y-%m-%d at %H:%M:%S UTC")
                ))
                .with_stop(true),
            None => ctx.say("No idea.").with_stop(true),
        }
    }
}

async fn retrieve_shared(
    store: &dyn FactoidStore,
    subject: &str,
    ctx: &Context,
    state: &mut StateSlot,
    direct: bool,
) -> CommandResult {
    let Ok(Some(factoid)) = store.find(subject).await else {
        return if direct {
            ctx.say(format!("I have no idea what '{subject}' could be."))
                .with_stop(true)
        } else {
            CommandResult::empty()
        };
    };

    if factoid.silent && !direct {
        return CommandResult::empty();
    }

    let facts = match store.facts(factoid.id).await {
        Ok(v) if !v.is_empty() => v,
        _ => {
            return if direct {
                ctx.say(format!("I have no idea what '{subject}' could be."))
                    .with_stop(true)
            } else {
                CommandResult::empty()
            };
        }
    };

    let last = facts.last().unwrap();
    let attribution = last.account_handle.clone();
    let when = last.created_at;

    state
        .with::<FactoidScratch, _, _>(|s| {
            s.who_said = attribution.clone();
            s.when_was = Some(when);
        })
        .await;

    let descriptions: Vec<String> = facts.into_iter().map(|f| f.description).collect();
    let joined = descriptions.join(" or ");
    let text = if let Some(reply_text) = joined.strip_prefix("<reply> ") {
        reply_text.to_string()
    } else if let Some(reply_text) = joined.strip_prefix("<reply>") {
        reply_text.trim().to_string()
    } else {
        let verb = if factoid.is_plural { "are" } else { "is" };
        format!("{} {} {}", factoid.subject, verb, joined)
    };
    let text = text.replace("<who>", &ctx.author.display);
    ctx.say(text).with_stop(true)
}

/// Listens for bare text and looks it up as a factoid. Lives at `Last` so it
/// only fires when no other command produced output.
pub struct FactoidListener {
    meta: CommandMeta,
    store: Arc<dyn FactoidStore>,
}

impl FactoidListener {
    pub fn new(store: Arc<dyn FactoidStore>) -> Self {
        Self {
            meta: CommandMeta::last_resort("factoid_listener", ""),
            store,
        }
    }
}

struct ListenerMatch(String);

#[async_trait]
impl Command for FactoidListener {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        let text = m.text.trim();
        if text.is_empty() {
            return None;
        }
        Some(MatchData::new(ListenerMatch(text.to_string())))
    }

    async fn handle(&self, m: MatchData, ctx: &Context, state: &mut StateSlot) -> CommandResult {
        let subject = match_data!(m => ListenerMatch);
        retrieve_shared(&*self.store, &subject.0, ctx, state, false).await
    }
}

fn error_reply(ctx: &Context, msg: &str, e: &dyn std::fmt::Display) -> CommandResult {
    tracing::warn!(error = %e, "factoid backend error");
    ctx.say(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatbot_core::testing::CommandTester;
    use whatbot_test_support::Pg;

    async fn cmd() -> (Factoid, Arc<dyn FactoidStore>) {
        let pg = Pg::shared().await;
        let store: Arc<dyn FactoidStore> = Arc::new(SqlFactoidStore::new(pg.fresh_store().await));
        let f = Factoid::new(store.clone());
        (f, store)
    }

    async fn cmd_with_db() -> (
        Factoid,
        Arc<dyn FactoidStore>,
        Arc<whatbot_storage::Store>,
    ) {
        let pg = Pg::shared().await;
        let db = pg.fresh_store().await;
        let store: Arc<dyn FactoidStore> = Arc::new(SqlFactoidStore::new(db.clone()));
        let f = Factoid::new(store.clone());
        (f, store, db)
    }

    async fn account(
        db: &whatbot_storage::Store,
        service: &str,
        handle: &str,
    ) -> whatbot_core::Account {
        db.accounts()
            .upsert(&whatbot_core::ServiceId::new(service), handle, handle)
            .await
            .expect("upsert account")
    }

    #[tokio::test]
    async fn assigns_quietly_in_public_channel_when_not_addressed() {
        let t = CommandTester::new().addressed(false);
        let (factoid, _store) = cmd().await;
        let r1 = t.say(&factoid, "rust is a systems language").await;
        assert!(r1.is_empty(), "public assignment is quiet, got {r1:?}");
        let r2 = t.say(&factoid, "what is rust").await;
        assert_eq!(r2, vec!["rust is a systems language".to_string()]);
    }

    #[tokio::test]
    async fn assigns_with_ack_when_addressed() {
        let t = CommandTester::new().with_author("nichelle");
        let (factoid, _store) = cmd().await;
        let r = t.say(&factoid, "rust is a systems language").await;
        assert_eq!(r, vec!["OK, nichelle.".to_string()]);
    }

    #[tokio::test]
    async fn handles_reply_directive() {
        let t = CommandTester::new();
        let (factoid, _store) = cmd().await;
        let _ = t.say(&factoid, "hello is <reply> hi there").await;
        let r = t.say(&factoid, "what is hello").await;
        assert_eq!(r, vec!["hi there".to_string()]);
    }

    #[tokio::test]
    async fn substitutes_who_placeholder() {
        let t = CommandTester::new().with_author("nichelle");
        let (factoid, _store) = cmd().await;
        let _ = t.say(&factoid, "greeting is <reply> hello <who>").await;
        let r = t.say(&factoid, "what is greeting").await;
        assert_eq!(r, vec!["hello nichelle".to_string()]);
    }

    #[tokio::test]
    async fn unknown_subject_says_no_idea() {
        let t = CommandTester::new();
        let (factoid, _store) = cmd().await;
        let r = t.say(&factoid, "what is nothing").await;
        assert_eq!(r.len(), 1);
        assert!(r[0].contains("no idea"), "got {r:?}");
    }

    #[tokio::test]
    async fn forget_removes_factoid() {
        let t = CommandTester::new();
        let (factoid, _store) = cmd().await;
        let _ = t.say(&factoid, "rust is fast").await;
        let r = t.say(&factoid, "forget rust").await;
        assert_eq!(r.len(), 1);
        let after = t.say(&factoid, "what is rust").await;
        assert!(after[0].contains("no idea"));
    }

    #[tokio::test]
    async fn who_said_returns_attribution_per_channel() {
        let (factoid, _store, db) = cmd_with_db().await;
        let nichelle = account(&db, "test", "nichelle").await;
        let leonard = account(&db, "test", "leonard").await;
        let t_a = CommandTester::new()
            .with_channel("a")
            .with_author_account(nichelle);
        let t_b = t_a.fork_with_channel("b").with_author_account(leonard);

        // Assignments in each channel — different attributions go through
        // the shared store.
        let _ = t_a.say(&factoid, "sky is blue").await;
        let _ = t_b.say(&factoid, "grass is green").await;

        // Trigger who_said tracking via retrieval in each channel.
        let _ = t_a.say(&factoid, "what is sky").await;
        let _ = t_b.say(&factoid, "what is grass").await;

        let q_a = t_a.say(&factoid, "who said that").await;
        let q_b = t_b.say(&factoid, "who said that").await;

        assert!(q_a[0].contains("nichelle"), "channel a: {q_a:?}");
        assert!(q_b[0].contains("leonard"), "channel b: {q_b:?}");
    }

    #[tokio::test]
    async fn who_said_persists_across_calls_in_same_channel() {
        let (factoid, _store, db) = cmd_with_db().await;
        let nichelle = account(&db, "test", "nichelle").await;
        let t = CommandTester::new().with_author_account(nichelle);
        let _ = t.say(&factoid, "sky is blue").await;
        let _ = t.say(&factoid, "what is sky").await;
        let r = t.say(&factoid, "who said that").await;
        assert!(r[0].contains("YOU"), "got {r:?}");
    }

    #[tokio::test]
    async fn who_said_isolated_across_services() {
        // Same channel name on different services must not share state.
        let (factoid, _store, db) = cmd_with_db().await;
        let nichelle = account(&db, "svc-one", "nichelle").await;
        let leonard = account(&db, "svc-two", "leonard").await;
        let t_one = CommandTester::new()
            .with_service("svc-one")
            .with_channel("general")
            .with_author_account(nichelle);
        let t_two = t_one
            .fork_with_service("svc-two")
            .with_author_account(leonard);

        let _ = t_one.say(&factoid, "sky is blue").await;
        let _ = t_two.say(&factoid, "grass is green").await;
        let _ = t_one.say(&factoid, "what is sky").await;
        let _ = t_two.say(&factoid, "what is grass").await;

        let q_one = t_one.say(&factoid, "who said that").await;
        let q_two = t_two.say(&factoid, "who said that").await;
        assert!(q_one[0].contains("nichelle"), "svc-one: {q_one:?}");
        assert!(q_two[0].contains("leonard"), "svc-two: {q_two:?}");
    }

    #[tokio::test]
    async fn who_said_recognizes_self() {
        let (factoid, _store, db) = cmd_with_db().await;
        let nichelle = account(&db, "test", "nichelle").await;
        let t = CommandTester::new().with_author_account(nichelle);
        let _ = t.say(&factoid, "sky is blue").await;
        let _ = t.say(&factoid, "what is sky").await;
        let r = t.say(&factoid, "who said that").await;
        assert!(r[0].contains("YOU"), "got {r:?}");
    }

    #[tokio::test]
    async fn assign_with_or_records_multiple_facts() {
        let t = CommandTester::new();
        let (factoid, store) = cmd().await;
        let _ = t.say(&factoid, "mood is happy or sad or neutral").await;
        let f = store.find("mood").await.unwrap().unwrap();
        let facts = store.facts(f.id).await.unwrap();
        let descs: Vec<String> = facts.into_iter().map(|f| f.description).collect();
        assert_eq!(
            descs,
            vec![
                "happy".to_string(),
                "sad".to_string(),
                "neutral".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn assigning_you_when_addressed_rewrites_to_bot_name() {
        // Old whatbot: "whatbot: you are great" stores the factoid under
        // the bot's display name, not the literal "you".
        let t = CommandTester::new()
            .with_author("nichelle")
            .with_bot("whatbot");
        let (factoid, store) = cmd().await;
        let _ = t.say(&factoid, "you are great").await;

        assert!(
            store.find("whatbot").await.unwrap().is_some(),
            "should have stored under the bot's name"
        );
        assert!(
            store.find("you").await.unwrap().is_none(),
            "should not have stored under literal 'you'"
        );
    }

    #[tokio::test]
    async fn assigning_you_when_not_addressed_uses_literal_you() {
        // Without addressing, "you are great" is just ambient chatter —
        // store it under "you" as the user said it. Mirrors old whatbot.
        let t = CommandTester::new().addressed(false);
        let (factoid, store) = cmd().await;
        let _ = t.say(&factoid, "you are great").await;

        assert!(
            store.find("you").await.unwrap().is_some(),
            "should store under literal 'you' when not addressed"
        );
    }

    #[tokio::test]
    async fn does_not_treat_question_as_assignment() {
        let t = CommandTester::new();
        let (factoid, store) = cmd().await;
        let _ = t.say(&factoid, "what is rust").await;
        // Should not have created a "what" factoid.
        assert!(store.find("what").await.unwrap().is_none());
    }

    async fn factoid_and_listener() -> (Factoid, FactoidListener, Arc<dyn FactoidStore>) {
        let pg = Pg::shared().await;
        let store: Arc<dyn FactoidStore> = Arc::new(SqlFactoidStore::new(pg.fresh_store().await));
        let f = Factoid::new(store.clone());
        let l = FactoidListener::new(store.clone());
        (f, l, store)
    }

    #[tokio::test]
    async fn listener_responds_to_bare_known_subject() {
        let t = CommandTester::new();
        let (f, l, _s) = factoid_and_listener().await;
        let _ = t.say(&f, "rust is a systems language").await;
        let r = t.say(&l, "rust").await;
        assert_eq!(r, vec!["rust is a systems language".to_string()]);
    }

    #[tokio::test]
    async fn listener_silent_for_unknown_subject() {
        let t = CommandTester::new();
        let (_f, l, _s) = factoid_and_listener().await;
        let r = t.say(&l, "nothing").await;
        assert!(
            r.is_empty(),
            "listener should be silent on unknown, got {r:?}"
        );
    }

    #[tokio::test]
    async fn listener_honors_reply_directive_in_bare_lookup() {
        let t = CommandTester::new();
        let (f, l, _s) = factoid_and_listener().await;
        let _ = t.say(&f, "ping is <reply> pong").await;
        let r = t.say(&l, "ping").await;
        assert_eq!(r, vec!["pong".to_string()]);
    }
}
