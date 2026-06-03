//! Seen: `seen <nick>` query.

use std::sync::Arc;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

use whatbot_core::{
    match_data, Command, CommandMeta, CommandResult, Context, Event, MatchData, Monitor, StateSlot,
};
use whatbot_storage::Store;

static RE_SEEN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^seen\s+(.+?)\s*$").unwrap());

pub struct SeenRecorder {
    store: Arc<Store>,
}

impl SeenRecorder {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Monitor for SeenRecorder {
    fn name(&self) -> &'static str {
        "seen_recorder"
    }

    async fn observe(&self, ctx: &Context, text: &str) {
        if RE_SEEN.is_match(text) {
            return;
        }
        if let Err(e) = self.store.seen().record(&ctx.author.display, text).await {
            tracing::warn!(error = %e, "seen record failed");
        }
    }
}

pub struct Seen {
    meta: CommandMeta,
    store: Arc<Store>,
}

impl Seen {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            meta: CommandMeta::core("seen", "seen <nick> — report when a user was last seen"),
            store,
        }
    }
}

struct SeenMatch {
    user: String,
}

#[async_trait]
impl Command for Seen {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else {
            return None;
        };
        let caps = RE_SEEN.captures(m.text.trim())?;
        let user = caps[1]
            .trim()
            .trim_end_matches(|c| matches!(c, '?' | '!' | '.'))
            .trim()
            .to_string();
        if user.is_empty() {
            return None;
        }
        Some(MatchData::new(SeenMatch { user }))
    }

    async fn handle(&self, m: MatchData, ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        let SeenMatch { user } = match_data!(m => SeenMatch);
        match self.store.seen().lookup(&user).await {
            Ok(Some(row)) => {
                let when = row.seen_at.format("%Y-%m-%d at %H:%M:%S UTC");
                ctx.say(format!(
                    "{} was last seen on {} saying, \"{}\".",
                    user, when, row.message
                ))
                .with_stop(true)
            }
            Ok(None) => ctx
                .say(format!("I have not seen {} yet.", user))
                .with_stop(true),
            Err(e) => {
                tracing::warn!(error = %e, "seen lookup failed");
                CommandResult::empty()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatbot_core::testing::CommandTester;
    use whatbot_test_support::Pg;

    async fn setup() -> (Seen, SeenRecorder, Arc<Store>) {
        let pg = Pg::shared().await;
        let store = pg.fresh_store().await;
        (
            Seen::new(store.clone()),
            SeenRecorder::new(store.clone()),
            store,
        )
    }

    #[tokio::test]
    async fn recorder_stores_message() {
        let (_, recorder, store) = setup().await;
        let t = CommandTester::new().with_author("nichelle");
        t.observe(&recorder, "hello there").await;
        let row = store.seen().lookup("nichelle").await.unwrap();
        assert!(row.is_some(), "should have recorded nichelle");
        assert_eq!(row.unwrap().message, "hello there");
    }

    #[tokio::test]
    async fn recorder_skips_seen_queries() {
        let (_, recorder, store) = setup().await;
        let t = CommandTester::new().with_author("nichelle");
        t.observe(&recorder, "seen bob").await;
        assert!(
            store.seen().lookup("nichelle").await.unwrap().is_none(),
            "seen queries must not be recorded"
        );
    }

    #[tokio::test]
    async fn recorder_ignores_empty_text() {
        let (_, recorder, store) = setup().await;
        let t = CommandTester::new().with_author("nichelle");
        t.observe(&recorder, "   ").await;
        assert!(store.seen().lookup("nichelle").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn seen_reports_known_user() {
        let (seen, _, store) = setup().await;
        store.seen().record("nichelle", "hey everyone").await.unwrap();
        let t = CommandTester::new();
        let replies = t.say(&seen, "seen nichelle").await;
        assert_eq!(replies.len(), 1);
        assert!(replies[0].contains("nichelle"), "{}", replies[0]);
        assert!(replies[0].contains("hey everyone"), "{}", replies[0]);
    }

    #[tokio::test]
    async fn seen_reports_unknown_user() {
        let (seen, _, _) = setup().await;
        let t = CommandTester::new();
        let replies = t.say(&seen, "seen nobody").await;
        assert_eq!(replies.len(), 1);
        assert!(replies[0].contains("have not seen"), "{}", replies[0]);
    }

    #[tokio::test]
    async fn seen_strips_trailing_punctuation() {
        let (seen, _, store) = setup().await;
        store.seen().record("nichelle", "hi").await.unwrap();
        let t = CommandTester::new();
        let replies = t.say(&seen, "seen nichelle?").await;
        assert!(
            replies[0].contains("hi"),
            "punctuation should be stripped: {}",
            replies[0]
        );
    }

    #[tokio::test]
    async fn seen_is_case_insensitive_command() {
        let (seen, _, store) = setup().await;
        store.seen().record("nichelle", "hi").await.unwrap();
        let t = CommandTester::new();
        let replies = t.say(&seen, "SEEN nichelle").await;
        assert_eq!(replies.len(), 1);
        assert!(replies[0].contains("nichelle"));
    }

    #[tokio::test]
    async fn seen_lookup_is_case_insensitive() {
        let (seen, _, store) = setup().await;
        store.seen().record("Nichelle", "hello").await.unwrap();
        let t = CommandTester::new();
        let replies = t.say(&seen, "seen nichelle").await;
        assert!(
            replies[0].contains("hello"),
            "lookup should be case-insensitive: {}",
            replies[0]
        );
    }

    #[tokio::test]
    async fn seen_ignores_unrelated_messages() {
        let (seen, _, _) = setup().await;
        let t = CommandTester::new();
        let replies = t.say(&seen, "hello world").await;
        assert!(replies.is_empty());
    }
}
