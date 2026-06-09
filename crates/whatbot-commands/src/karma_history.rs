//! KarmaHistory: queries over the karma event log

use std::sync::Arc;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

use whatbot_core::{
    match_data, Command, CommandMeta, CommandResult, Context, Event, MatchData, StateSlot,
};
use whatbot_storage::Store;

static RE_RANDOM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(\w+) (like|hate)s what\??$").unwrap()
});
static RE_CONTROVERSY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^[. ]*fightin[g']? words\??$").unwrap()
});
static RE_EXTREMES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^[. ]*what(?:'s| is)(?: the)? (best|worst)\??$").unwrap()
});
static RE_WHO: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^[. ]*who (hates|likes|loves|doesn't like|plussed|minused) (.+)").unwrap()
});

fn clean_subject(raw: &str) -> String {
    let s = raw.trim().trim_end_matches(|c| matches!(c, '.' | '?' | '!'));
    let s = s.trim();
    for article in &["the ", "a ", "an "] {
        if s.to_lowercase().starts_with(article) {
            return s[article.len()..].to_string();
        }
    }
    s.to_string()
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

enum Action {
    RandomExclusive { who: String, positive: bool },
    Controversial,
    Extremes { best: bool },
    WhoVoted { subject: String, positive: bool },
}

pub struct KarmaHistory {
    meta: CommandMeta,
    store: Arc<Store>,
}

impl KarmaHistory {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            meta: CommandMeta::extension(
                "karma_history",
                "karma trivia: who likes/hates X, what's the best/worst, fighting words",
            ),
            store,
        }
    }
}

#[async_trait]
impl Command for KarmaHistory {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        let text = m.text.trim();

        if let Some(caps) = RE_RANDOM.captures(text) {
            let who = caps[1].to_string();
            let positive = caps[2].eq_ignore_ascii_case("like");
            return Some(MatchData::new(Action::RandomExclusive { who, positive }));
        }
        if RE_CONTROVERSY.is_match(text) {
            return Some(MatchData::new(Action::Controversial));
        }
        if let Some(caps) = RE_EXTREMES.captures(text) {
            let best = caps[1].eq_ignore_ascii_case("best");
            return Some(MatchData::new(Action::Extremes { best }));
        }
        if let Some(caps) = RE_WHO.captures(text) {
            let verb = caps[1].to_lowercase();
            let positive = matches!(verb.as_str(), "likes" | "loves" | "plussed");
            let subject = clean_subject(&caps[2]);
            return Some(MatchData::new(Action::WhoVoted { subject, positive }));
        }
        None
    }

    async fn handle(&self, m: MatchData, ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        let action = match_data!(m => Action);
        match action {
            Action::RandomExclusive { who, positive } => {
                let verb = if positive { "like" } else { "hate" };
                match self.store.karma().random_exclusive(&who, positive).await {
                    Ok(Some(subject)) => {
                        ctx.say(format!("{who} {verb}s {subject}.")).with_stop(true)
                    }
                    Ok(None) => ctx
                        .say(format!(
                            "{who} doesn't {verb} anything weird. that I know of."
                        ))
                        .with_stop(true),
                    Err(e) => {
                        tracing::warn!(error = %e, "karma_history random_exclusive failed");
                        CommandResult::empty()
                    }
                }
            }
            Action::Controversial => {
                match self.store.karma().controversial_subjects(10).await {
                    Ok(subjects) if subjects.is_empty() => {
                        ctx.say("No karma data yet.").with_stop(true)
                    }
                    Ok(subjects) => {
                        let parts: Vec<String> = subjects
                            .into_iter()
                            .map(|s| format!("{} ({})", s.subject, s.score))
                            .collect();
                        ctx.say(parts.join(", ")).with_stop(true)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "karma_history controversial failed");
                        CommandResult::empty()
                    }
                }
            }
            Action::Extremes { best } => {
                let label = if best { "best" } else { "worst" };
                match self.store.karma().top_subjects(!best, 10).await {
                    Ok(subjects) if subjects.is_empty() => {
                        ctx.say("No karma data yet.").with_stop(true)
                    }
                    Ok(subjects) => {
                        let parts: Vec<String> = subjects
                            .into_iter()
                            .map(|s| format!("{} ({})", s.subject, s.score))
                            .collect();
                        ctx.say(format!("The {label} of everything is: {}", parts.join(", ")))
                            .with_stop(true)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "karma_history extremes failed");
                        CommandResult::empty()
                    }
                }
            }
            Action::WhoVoted { subject, positive } => {
                let requester = ctx.author.display.clone();
                match self.store.karma().scores_for_subject(&subject, positive).await {
                    Ok(voters) if voters.is_empty() => {
                        ctx.say(format!("{requester}: Nobody!")).with_stop(true)
                    }
                    Ok(voters) if voters.len() == 1 => {
                        let v = &voters[0];
                        let count = v.total.unsigned_abs();
                        let howmuch = match count {
                            1 => "once".to_string(),
                            2 => "twice".to_string(),
                            n => format!("{n} times"),
                        };
                        let who = &v.display;
                        if who.eq_ignore_ascii_case(&requester) {
                            ctx.say(format!(
                                "{requester}: It was YOU! {}.",
                                capitalize(&howmuch)
                            ))
                            .with_stop(true)
                        } else {
                            ctx.say(format!("{requester}: It was {who}, {howmuch}."))
                                .with_stop(true)
                        }
                    }
                    Ok(voters) => {
                        let total: i64 = voters.iter().map(|v| v.total).sum();
                        let parts: Vec<String> = voters
                            .iter()
                            .map(|v| format!("{} ({})", v.display, v.total))
                            .collect();
                        ctx.say(format!("{requester}: {} = {total}", parts.join(", ")))
                            .with_stop(true)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "karma_history who_voted failed");
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

    async fn cmd() -> (KarmaHistory, Arc<Store>) {
        let pg = Pg::shared().await;
        let store = pg.fresh_store().await;
        (KarmaHistory::new(store.clone()), store)
    }

    #[tokio::test]
    async fn extreme_best_returns_sorted() {
        let t = CommandTester::new();
        let (kh, store) = cmd().await;
        for _ in 0..3 {
            store.karma().record("freebsd", 1, None).await.unwrap();
        }
        store.karma().record("solaris", 1, None).await.unwrap();
        let r = t.say(&kh, "what's the best?").await;
        assert_eq!(r.len(), 1);
        assert!(r[0].starts_with("The best of everything is:"), "{}", r[0]);
        assert!(r[0].contains("freebsd (3)"), "{}", r[0]);
        assert!(r[0].contains("solaris (1)"), "{}", r[0]);
        assert!(r[0].find("freebsd").unwrap() < r[0].find("solaris").unwrap(), "{}", r[0]);
    }

    #[tokio::test]
    async fn extreme_worst_returns_sorted() {
        let t = CommandTester::new();
        let (kh, store) = cmd().await;
        store.karma().record("javascript", -1, None).await.unwrap();
        store.karma().record("javascript", -1, None).await.unwrap();
        store.karma().record("php", -1, None).await.unwrap();
        let r = t.say(&kh, "what's the worst").await;
        assert_eq!(r.len(), 1);
        assert!(r[0].starts_with("The worst of everything is:"), "{}", r[0]);
        assert!(r[0].find("javascript").unwrap() < r[0].find("php").unwrap(), "{}", r[0]);
    }

    #[tokio::test]
    async fn no_match_on_unrelated_text() {
        let t = CommandTester::new();
        let (kh, _store) = cmd().await;
        let r = t.say(&kh, "hello there").await;
        assert!(r.is_empty());
    }

    #[tokio::test]
    async fn controversy_prefers_mixed_votes() {
        let t = CommandTester::new();
        let (kh, store) = cmd().await;

        // Mixed votes => higher controversy score
        store.karma().record("rust", 1, None).await.unwrap();
        store.karma().record("rust", -1, None).await.unwrap();

        // One-sided votes => low/zero controversy score
        store.karma().record("php", -1, None).await.unwrap();
        store.karma().record("php", -1, None).await.unwrap();

        let r = t.say(&kh, "fighting words?").await;
        assert_eq!(r.len(), 1);
        let line = &r[0];
        assert!(line.contains("rust (2)"), "{line}");
        assert!(line.find("rust").unwrap() < line.find("php").unwrap(), "{line}");
    }
}
