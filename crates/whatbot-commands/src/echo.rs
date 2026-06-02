use async_trait::async_trait;

use whatbot_core::{
    match_data, Command, CommandMeta, CommandResult, Context, Event, MatchData, StateSlot,
};

pub struct Echo {
    meta: CommandMeta,
}

impl Echo {
    pub fn new() -> Self {
        Self {
            meta: CommandMeta::core("echo", "echo <text> — repeats text back"),
        }
    }
}

impl Default for Echo {
    fn default() -> Self {
        Self::new()
    }
}

struct EchoMatch {
    payload: String,
}

#[async_trait]
impl Command for Echo {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        let text = m.text.trim();
        let rest = text.strip_prefix("echo ")?.trim();
        if rest.is_empty() {
            return None;
        }
        Some(MatchData::new(EchoMatch {
            payload: rest.to_string(),
        }))
    }

    async fn handle(&self, m: MatchData, ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        let echo = match_data!(m => EchoMatch);
        ctx.say(&echo.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatbot_core::testing::CommandTester;

    #[tokio::test]
    async fn echoes_payload_in_channel() {
        let t = CommandTester::new();
        let replies = t.say(&Echo::new(), "echo hello world").await;
        assert_eq!(replies, vec!["hello world".to_string()]);
    }

    #[tokio::test]
    async fn ignores_unrelated_text() {
        let t = CommandTester::new();
        let replies = t.say(&Echo::new(), "what is rust").await;
        assert!(replies.is_empty());
    }

    #[tokio::test]
    async fn ignores_bare_echo_keyword() {
        let t = CommandTester::new();
        let replies = t.say(&Echo::new(), "echo").await;
        assert!(replies.is_empty());
    }

    #[tokio::test]
    async fn reply_goes_to_originating_channel() {
        let t = CommandTester::new().with_channel("offtopic");
        let replies = t
            .send(&Echo::new(), "echo over here")
            .await
            .unwrap_replies();
        assert_eq!(replies.len(), 1);
        match &replies[0].destination {
            whatbot_core::Destination::Channel { channel, .. } => {
                assert_eq!(channel.as_str(), "offtopic");
            }
            other => panic!("unexpected destination: {other:?}"),
        }
    }

    #[tokio::test]
    async fn ctx_mention_uses_configured_renderer() {
        use async_trait::async_trait;
        use whatbot_core::{
            match_data, Account, Command, CommandMeta, CommandResult, Context, Event, MatchData,
            MentionRenderer, StateSlot,
        };

        /// Renderer that wraps the display name in `<>`.
        #[derive(Debug)]
        struct TestMentionRenderer;
        impl MentionRenderer for TestMentionRenderer {
            fn render(&self, account: &Account) -> String {
                format!("<{}>", account.display)
            }
        }

        /// Command that mentions its author and replies with the string.
        struct MentionMe;
        #[async_trait]
        impl Command for MentionMe {
            fn meta(&self) -> &CommandMeta {
                static M: std::sync::OnceLock<CommandMeta> = std::sync::OnceLock::new();
                M.get_or_init(|| CommandMeta::core("mentionme", ""))
            }
            fn matches(&self, _e: &Event, _c: &Context) -> Option<MatchData> {
                Some(MatchData::new(()))
            }
            async fn handle(
                &self,
                m: MatchData,
                ctx: &Context,
                _s: &mut StateSlot,
            ) -> CommandResult {
                let _ = match_data!(m => ());
                let s = format!("hello {}", ctx.mention(&ctx.author));
                CommandResult::reply(ctx.reply_here(s))
            }
        }

        let t = CommandTester::new()
            .with_author("nichelle")
            .with_mention_renderer(TestMentionRenderer);
        let r = t.say(&MentionMe, "hi").await;
        assert_eq!(r, vec!["hello <nichelle>".to_string()]);
    }

    #[tokio::test]
    async fn require_direct_blocks_via_tester() {
        use async_trait::async_trait;
        use whatbot_core::{
            Command, CommandMeta, CommandResult, Context, Event, MatchData, Priority, StateSlot,
        };

        struct Strict;
        #[async_trait]
        impl Command for Strict {
            fn meta(&self) -> &CommandMeta {
                static META: std::sync::OnceLock<CommandMeta> = std::sync::OnceLock::new();
                META.get_or_init(|| CommandMeta {
                    name: "strict",
                    priority: Priority::Core,
                    require_direct: true,
                    required_caps: Vec::new(),
                    help: "",
                })
            }
            fn matches(&self, _evt: &Event, _ctx: &Context) -> Option<MatchData> {
                Some(MatchData::new(()))
            }
            async fn handle(
                &self,
                _m: MatchData,
                ctx: &Context,
                _s: &mut StateSlot,
            ) -> CommandResult {
                CommandResult::reply(ctx.reply_here("ack"))
            }
        }

        let undirected = CommandTester::new().addressed(false);
        assert!(
            undirected.send(&Strict, "hi").await.is_denied(),
            "require_direct=true must produce Denied (not NoMatch)"
        );

        let directed = CommandTester::new().addressed(true);
        let replies = directed.send(&Strict, "hi").await.unwrap_replies();
        assert_eq!(replies.len(), 1);
    }
}
