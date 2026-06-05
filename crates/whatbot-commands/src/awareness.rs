use async_trait::async_trait;
use once_cell::sync::Lazy;
use rand::seq::SliceRandom;
use regex::Regex;

use whatbot_core::{
    match_data, Command, CommandMeta, CommandResult, Context, Event, MatchData, StateSlot,
};

static RE_GREETING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(hey|hi|hello|word|sup|morning|good morning)[?!. ]*$").unwrap()
});

const GREETINGS: &[&str] = &[
    "hey", "sup", "what's up", "yo", "word", "hi", "hello", "greetings", "allo", "ayyy",
];

enum Action {
    JustName,
    Greeting,
}

pub struct Awareness {
    meta: CommandMeta,
}

impl Awareness {
    pub fn new() -> Self {
        Self {
            meta: CommandMeta::core("awareness", "").require_direct(),
        }
    }
}

impl Default for Awareness {
    fn default() -> Self {
        Self::new()
    }
}

fn is_just_bot_name(text: &str, bot_name: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    let lower = text.to_lowercase();
    let name = bot_name.to_lowercase();
    if !lower.starts_with(&name) {
        return false;
    }
    lower[name.len()..]
        .trim_matches(|c| matches!(c, '?' | '!' | '.'))
        .is_empty()
}

#[async_trait]
impl Command for Awareness {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else {
            return None;
        };
        let text = m.text.trim();

        if is_just_bot_name(text, &ctx.bot.display) {
            return Some(MatchData::new(Action::JustName));
        }
        if RE_GREETING.is_match(text) {
            return Some(MatchData::new(Action::Greeting));
        }
        None
    }

    async fn handle(&self, m: MatchData, ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        let action = match_data!(m => Action);
        match action {
            Action::JustName => ctx.say("what").with_stop(true),
            Action::Greeting => {
                let greeting = GREETINGS
                    .choose(&mut rand::thread_rng())
                    .copied()
                    .unwrap_or("hello");
                ctx.say(format!("{}, {}.", greeting, ctx.mention(&ctx.author)))
                    .with_stop(true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatbot_core::testing::CommandTester;

    fn cmd() -> Awareness {
        Awareness::new()
    }

    #[tokio::test]
    async fn replies_what_to_just_the_bot_name() {
        let t = CommandTester::new().with_bot("whatbot");
        let r = t.say(&cmd(), "whatbot").await;
        assert_eq!(r, vec!["what".to_string()]);
    }

    #[tokio::test]
    async fn greets_back_when_addressed_with_hi() {
        let t = CommandTester::new().with_author("nichelle");
        let r = t.say(&cmd(), "hi").await;
        assert_eq!(r.len(), 1);
        assert!(r[0].ends_with('.'), "reply should end with period: {}", r[0]);
        assert!(r[0].contains("nichelle"), "reply should mention the author: {}", r[0]);
    }
}
