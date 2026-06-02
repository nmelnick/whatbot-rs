//! Karma: `subject++` / `subject--` and `karma <subject>`.

pub mod adapter;
pub mod store;

use std::sync::Arc;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

use whatbot_core::{
    match_data, Command, CommandMeta, CommandResult, Context, Event, MatchData, StateSlot,
};

pub use adapter::SqlKarmaStore;
pub use store::{KarmaStore, KarmaStoreError};

/// Matches `subject++` or `subject--`.
static RE_KARMA: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)(?:^|\s)(?:\(([^)]+)\)|([A-Za-z0-9_\-.]+?))(\+\+|--)(?:\s|$|[!.?,])").unwrap()
});

static RE_QUERY: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^karma\s+(.+?)\s*\??$").unwrap());

enum Action {
    Apply { subject: String, delta: i32 },
    Query(String),
}

pub struct Karma {
    meta: CommandMeta,
    store: Arc<dyn KarmaStore>,
}

impl Karma {
    pub fn new(store: Arc<dyn KarmaStore>) -> Self {
        Self {
            meta: CommandMeta::core("karma", "subject++ / subject-- ; karma <subject>"),
            store,
        }
    }
}

#[async_trait]
impl Command for Karma {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        let text = m.text.trim();

        if let Some(caps) = RE_QUERY.captures(text) {
            return Some(MatchData::new(Action::Query(caps[1].trim().to_string())));
        }

        // The increment/decrement regex matches the first occurrence
        if let Some(caps) = RE_KARMA.captures(text) {
            let subject = caps
                .get(1)
                .or_else(|| caps.get(2))
                .map(|m| m.as_str().trim().to_string())?;
            if subject.is_empty() {
                return None;
            }
            let delta = if &caps[3] == "++" { 1 } else { -1 };
            return Some(MatchData::new(Action::Apply { subject, delta }));
        }
        None
    }

    async fn handle(&self, m: MatchData, ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        let action = match_data!(m => Action);
        match action {
            Action::Query(subject) => {
                let score = self.store.score(&subject).await.ok().flatten().unwrap_or(0);
                ctx.say(format!("{subject} has karma of {score}."))
                    .with_stop(true)
            }
            Action::Apply { subject, delta } => {
                // No self-karma.
                if ctx.author.matches_handle(&subject) {
                    return ctx
                        .say(format!("{}: you can't karma yourself.", ctx.author.display))
                        .with_stop(true);
                }
                match self.store.apply(&subject, delta, Some(&ctx.author)).await {
                    Ok(new_score) => {
                        tracing::info!("subject {subject} set karma to {new_score}.");
                        CommandResult::handled_silently()
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "karma apply failed");
                        CommandResult::empty()
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatbot_core::testing::CommandTester;
    use whatbot_test_support::Pg;

    async fn cmd() -> (Karma, Arc<dyn KarmaStore>) {
        let pg = Pg::shared().await;
        let store: Arc<dyn KarmaStore> = Arc::new(SqlKarmaStore::new(pg.fresh_store().await));
        let k = Karma::new(store.clone());
        (k, store)
    }

    #[tokio::test]
    async fn increment_starts_at_one() {
        let t = CommandTester::new();
        let (k, store) = cmd().await;
        let r = t.say(&k, "rust++").await;
        assert!(r.is_empty(), "karma apply is silent in channel: {r:?}");
        assert_eq!(store.score("rust").await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn decrement_goes_negative() {
        let t = CommandTester::new();
        let (k, store) = cmd().await;
        let r = t.say(&k, "javascript--").await;
        assert!(r.is_empty());
        assert_eq!(store.score("javascript").await.unwrap(), Some(-1));
    }

    #[tokio::test]
    async fn karma_query_reports_score() {
        let t = CommandTester::new();
        let (k, _s) = cmd().await;
        let _ = t.say(&k, "rust++").await;
        let _ = t.say(&k, "rust++").await;
        let r = t.say(&k, "karma rust").await;
        assert_eq!(r, vec!["rust has karma of 2.".to_string()]);
    }

    #[tokio::test]
    async fn karma_query_unknown_subject_is_zero() {
        let t = CommandTester::new();
        let (k, _s) = cmd().await;
        let r = t.say(&k, "karma nothing").await;
        assert_eq!(r, vec!["nothing has karma of 0.".to_string()]);
    }

    #[tokio::test]
    async fn cannot_karma_self() {
        let t = CommandTester::new().with_author("nichelle");
        let (k, store) = cmd().await;
        let r = t.say(&k, "nichelle++").await;
        assert!(r[0].contains("can't karma yourself"));
        assert_eq!(store.score("nichelle").await.unwrap(), None);
    }

    #[tokio::test]
    async fn karma_matches_inline_in_message() {
        let t = CommandTester::new();
        let (k, store) = cmd().await;
        let r = t.say(&k, "I love rust++ today").await;
        assert!(r.is_empty());
        assert_eq!(store.score("rust").await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn ignores_unrelated_plusplus() {
        let t = CommandTester::new();
        let (k, _s) = cmd().await;
        let r = t.say(&k, "this has no karma in it").await;
        assert!(r.is_empty());
    }

    #[tokio::test]
    async fn parenthesized_multi_word_subject() {
        let t = CommandTester::new();
        let (k, store) = cmd().await;
        let r = t.say(&k, "(steve jobs)++").await;
        assert!(r.is_empty(), "karma is silent in channel: {r:?}");
        assert_eq!(store.score("steve jobs").await.unwrap(), Some(1));

        let q = t.say(&k, "karma steve jobs").await;
        assert_eq!(q, vec!["steve jobs has karma of 1.".to_string()]);
    }

    #[tokio::test]
    async fn parenthesized_decrement() {
        let t = CommandTester::new();
        let (k, store) = cmd().await;
        let _ = t.say(&k, "(visual basic)--").await;
        assert_eq!(store.score("visual basic").await.unwrap(), Some(-1));
    }

    #[tokio::test]
    async fn parenthesized_inline_in_message() {
        let t = CommandTester::new();
        let (k, store) = cmd().await;
        let _ = t.say(&k, "I think (steve jobs)++ today").await;
        assert_eq!(store.score("steve jobs").await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn parenthesized_trims_inner_whitespace() {
        // `(  steve jobs  )++` should still karma "steve jobs", not the
        // padded form.
        let t = CommandTester::new();
        let (k, store) = cmd().await;
        let _ = t.say(&k, "(  steve jobs  )++").await;
        assert_eq!(store.score("steve jobs").await.unwrap(), Some(1));
        assert_eq!(store.score("  steve jobs  ").await.unwrap(), None);
    }

    #[tokio::test]
    async fn parenthesized_supports_punctuation() {
        // The bare form rejects `+` chars; the parens form passes them through.
        let t = CommandTester::new();
        let (k, store) = cmd().await;
        let _ = t.say(&k, "(C++)++").await;
        assert_eq!(store.score("c++").await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn parenthesized_self_karma_rejected() {
        let t = CommandTester::new().with_author("steve jobs");
        let (k, store) = cmd().await;
        let r = t.say(&k, "(steve jobs)++").await;
        assert!(r[0].contains("can't karma yourself"));
        assert_eq!(store.score("steve jobs").await.unwrap(), None);
    }
}
