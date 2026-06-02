//! Auto-generated help. Provides two patterns:
//!
//! * `help` — list every installed command with a non-empty `help` blurb.
//! * `help <name>` — detail one command (priority, caps, direct-only,
//!   help text).

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

use whatbot_core::{
    match_data, Capability, Command, CommandMeta, CommandResult, Context, Event, MatchData,
    Priority, Registry, StateSlot,
};

static RE_HELP_DETAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^help\s+(.+?)\s*$").unwrap());

/// Per-command snapshot taken at Help construction time.
#[derive(Debug, Clone)]
struct HelpEntry {
    name: &'static str,
    help: &'static str,
    priority: Priority,
    require_direct: bool,
    required_caps: Vec<Capability>,
}

impl HelpEntry {
    fn from_meta(m: &CommandMeta) -> Self {
        Self {
            name: m.name,
            help: m.help,
            priority: m.priority,
            require_direct: m.require_direct,
            required_caps: m.required_caps.clone(),
        }
    }

    /// Skip commands with no help blurb
    fn is_listable(&self) -> bool {
        !self.help.is_empty()
    }
}

enum Action {
    List,
    Detail(String),
}

pub struct Help {
    meta: CommandMeta,
    entries: Vec<HelpEntry>,
}

impl Help {
    /// Snapshot the Command registry
    pub fn from_registry(registry: &Registry) -> Self {
        let mut entries: Vec<HelpEntry> = registry
            .iter_commands()
            .map(|c| HelpEntry::from_meta(c.meta()))
            .collect();

        let meta = CommandMeta::core("help", "help / help <name> — list commands or describe one");
        entries.push(HelpEntry::from_meta(&meta));

        entries.sort_by(|a, b| (a.priority as u8, a.name).cmp(&(b.priority as u8, b.name)));

        Self { meta, entries }
    }

    fn render_list(&self) -> String {
        let mut out = String::from("Available commands:");
        for e in self.entries.iter().filter(|e| e.is_listable()) {
            out.push_str(&format!("\n  {} — {}", e.name, e.help));
        }
        out.push_str("\nUse `help <name>` for details about a specific command.");
        out
    }

    fn render_detail(&self, name: &str) -> String {
        let needle = name.trim();
        let Some(e) = self
            .entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(needle))
        else {
            return format!("No command named `{name}`. Try `help` for the full list.");
        };
        let mut out = format!("{} ({:?}): {}", e.name, e.priority, e.help);
        if e.require_direct {
            out.push_str("\n  Requires addressing the bot directly.");
        }
        if !e.required_caps.is_empty() {
            out.push_str("\n  Required capabilities:");
            for cap in &e.required_caps {
                out.push_str(&format!(" {cap:?}"));
            }
        }
        out
    }
}

#[async_trait]
impl Command for Help {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        let text = m.text.trim();
        if text.eq_ignore_ascii_case("help") {
            return Some(MatchData::new(Action::List));
        }
        if let Some(caps) = RE_HELP_DETAIL.captures(text) {
            return Some(MatchData::new(Action::Detail(caps[1].to_string())));
        }
        None
    }

    async fn handle(&self, m: MatchData, ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        let action = match_data!(m => Action);
        let text = match action {
            Action::List => self.render_list(),
            Action::Detail(name) => self.render_detail(&name),
        };
        ctx.say(text).with_stop(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use whatbot_core::testing::CommandTester;
    use whatbot_core::Registry;

    use crate::echo::Echo;

    fn registry_with(cmds: Vec<Arc<dyn Command>>) -> Registry {
        let mut r = Registry::new();
        for c in cmds {
            r.install(c);
        }
        r
    }

    #[tokio::test]
    async fn list_includes_installed_commands_and_help_itself() {
        let r = registry_with(vec![Arc::new(Echo::new())]);
        let help = Help::from_registry(&r);
        let t = CommandTester::new();
        let replies = t.say(&help, "help").await;
        assert_eq!(replies.len(), 1);
        let body = &replies[0];
        assert!(body.contains("echo"), "should list echo: {body}");
        assert!(body.contains("help"), "should list itself: {body}");
    }

    #[tokio::test]
    async fn list_hides_commands_with_empty_help() {
        use async_trait::async_trait;

        struct Hidden;
        #[async_trait]
        impl Command for Hidden {
            fn meta(&self) -> &CommandMeta {
                static M: std::sync::OnceLock<CommandMeta> = std::sync::OnceLock::new();
                M.get_or_init(|| CommandMeta::core("hidden", ""))
            }
            fn matches(&self, _: &Event, _: &Context) -> Option<MatchData> {
                None
            }
            async fn handle(&self, _: MatchData, _: &Context, _: &mut StateSlot) -> CommandResult {
                CommandResult::empty()
            }
        }

        let r = registry_with(vec![Arc::new(Echo::new()), Arc::new(Hidden)]);
        let help = Help::from_registry(&r);
        let t = CommandTester::new();
        let replies = t.say(&help, "help").await;
        let body = &replies[0];
        assert!(body.contains("echo"));
        assert!(
            !body.contains("hidden"),
            "empty-help command must be filtered: {body}"
        );
    }

    #[tokio::test]
    async fn detail_returns_command_specific_help() {
        let r = registry_with(vec![Arc::new(Echo::new())]);
        let help = Help::from_registry(&r);
        let t = CommandTester::new();
        let replies = t.say(&help, "help echo").await;
        assert_eq!(replies.len(), 1);
        let body = &replies[0];
        assert!(body.contains("echo"));
        assert!(
            body.contains("repeats text back"),
            "help text should appear: {body}"
        );
    }

    #[tokio::test]
    async fn detail_is_case_insensitive() {
        let r = registry_with(vec![Arc::new(Echo::new())]);
        let help = Help::from_registry(&r);
        let t = CommandTester::new();
        let replies = t.say(&help, "help ECHO").await;
        assert!(replies[0].contains("repeats text back"));
    }

    #[tokio::test]
    async fn detail_for_unknown_command_says_so() {
        let r = registry_with(vec![Arc::new(Echo::new())]);
        let help = Help::from_registry(&r);
        let t = CommandTester::new();
        let replies = t.say(&help, "help nonsense").await;
        assert!(replies[0].contains("No command named"));
    }

    #[tokio::test]
    async fn ignores_unrelated_text() {
        let r = registry_with(vec![]);
        let help = Help::from_registry(&r);
        let t = CommandTester::new();
        let replies = t.say(&help, "rust is great").await;
        assert!(replies.is_empty());
    }
}
