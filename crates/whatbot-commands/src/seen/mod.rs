//! Seen: `seen <nick>` query.

pub mod adapter;
pub mod store;

use std::sync::Arc;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

use whatbot_core::{
    match_data, Command, CommandMeta, CommandResult, Context, Event, MatchData, StateSlot,
};

pub use adapter::SqlSeenStore;
pub use store::{SeenRecord, SeenStore, SeenStoreError};

static RE_SEEN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^seen\s+(.+?)\s*$").unwrap());

pub struct SeenRecorder {
    meta: CommandMeta,
    store: Arc<dyn SeenStore>,
}

impl SeenRecorder {
    pub fn new(store: Arc<dyn SeenStore>) -> Self {
        Self {
            meta: CommandMeta::primary("seen_recorder", ""),
            store,
        }
    }
}

struct RecordMatch {
    display: String,
    text: String,
}

#[async_trait]
impl Command for SeenRecorder {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else {
            return None;
        };
        let text = m.text.trim();
        if text.is_empty() || RE_SEEN.is_match(text) {
            return None;
        }
        Some(MatchData::new(RecordMatch {
            display: ctx.author.display.clone(),
            text: text.to_string(),
        }))
    }

    async fn handle(&self, m: MatchData, _ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        let rm = match_data!(m => RecordMatch);
        if let Err(e) = self.store.record(&rm.display, &rm.text).await {
            tracing::warn!(error = %e, "seen record failed");
        }
        CommandResult::empty()
    }
}

pub struct Seen {
    meta: CommandMeta,
    store: Arc<dyn SeenStore>,
}

impl Seen {
    pub fn new(store: Arc<dyn SeenStore>) -> Self {
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
        match self.store.lookup(&user).await {
            Ok(Some(record)) => {
                let when = record.seen_at.format("%Y-%m-%d at %H:%M:%S UTC");
                ctx.say(format!(
                    "{} was last seen on {} saying, \"{}\".",
                    user, when, record.message
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
    use std::collections::HashMap;
    use std::sync::Mutex;

    use whatbot_core::testing::CommandTester;

    struct MockSeenStore {
        data: Mutex<HashMap<String, SeenRecord>>,
    }

    impl MockSeenStore {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                data: Mutex::new(HashMap::new()),
            })
        }
    }

    #[async_trait]
    impl SeenStore for MockSeenStore {
        async fn record(&self, handle: &str, message: &str) -> Result<(), SeenStoreError> {
            self.data.lock().unwrap().insert(
                handle.to_lowercase(),
                SeenRecord {
                    handle: handle.to_string(),
                    message: message.to_string(),
                    seen_at: chrono::Utc::now(),
                },
            );
            Ok(())
        }

        async fn lookup(&self, handle: &str) -> Result<Option<SeenRecord>, SeenStoreError> {
            Ok(self
                .data
                .lock()
                .unwrap()
                .get(&handle.to_lowercase())
                .cloned())
        }
    }

    #[tokio::test]
    async fn recorder_stores_message() {
        let store = MockSeenStore::new();
        let t = CommandTester::new().with_author("alice");
        let recorder = SeenRecorder::new(store.clone());
        let replies = t.say(&recorder, "hello there").await;
        assert!(replies.is_empty(), "recorder must be silent");
        let rec = store.lookup("alice").await.unwrap();
        assert!(rec.is_some(), "should have recorded alice");
        assert_eq!(rec.unwrap().message, "hello there");
    }

    #[tokio::test]
    async fn seen_reports_known_user() {
        let store = MockSeenStore::new();
        store.record("alice", "hey everyone").await.unwrap();
        let t = CommandTester::new();
        let seen = Seen::new(store.clone());
        let replies = t.say(&seen, "seen alice").await;
        assert_eq!(replies.len(), 1);
        assert!(replies[0].contains("alice"), "should mention user: {}", replies[0]);
        assert!(replies[0].contains("hey everyone"), "should include message: {}", replies[0]);
    }

    #[tokio::test]
    async fn seen_reports_unknown_user() {
        let store = MockSeenStore::new();
        let t = CommandTester::new();
        let seen = Seen::new(store.clone());
        let replies = t.say(&seen, "seen nobody").await;
        assert_eq!(replies.len(), 1);
        assert!(
            replies[0].contains("have not seen"),
            "should say not seen: {}",
            replies[0]
        );
    }

    #[tokio::test]
    async fn seen_strips_trailing_punctuation() {
        let store = MockSeenStore::new();
        store.record("alice", "hi").await.unwrap();
        let t = CommandTester::new();
        let seen = Seen::new(store.clone());
        let replies = t.say(&seen, "seen alice?").await;
        assert!(replies[0].contains("alice"), "punctuation should be stripped: {}", replies[0]);
        assert!(!replies[0].contains("have not seen"), "should have found alice: {}", replies[0]);
    }

    #[tokio::test]
    async fn seen_is_case_insensitive_command() {
        let store = MockSeenStore::new();
        store.record("alice", "hi").await.unwrap();
        let t = CommandTester::new();
        let seen = Seen::new(store.clone());
        let replies = t.say(&seen, "SEEN alice").await;
        assert_eq!(replies.len(), 1);
        assert!(replies[0].contains("alice"));
    }

    #[tokio::test]
    async fn seen_lookup_is_case_insensitive() {
        let store = MockSeenStore::new();
        store.record("Alice", "hello").await.unwrap();
        let t = CommandTester::new();
        let seen = Seen::new(store.clone());
        let replies = t.say(&seen, "seen alice").await;
        assert!(replies[0].contains("hello"), "lookup should be case-insensitive: {}", replies[0]);
    }

    #[tokio::test]
    async fn seen_ignores_unrelated_messages() {
        let store = MockSeenStore::new();
        let t = CommandTester::new();
        let seen = Seen::new(store.clone());
        let replies = t.say(&seen, "hello world").await;
        assert!(replies.is_empty());
    }
}
