//! Admin: bot admin commands

use std::sync::Arc;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

use whatbot_core::{
    match_data, Capability, Command, CommandMeta, CommandResult, Context, Event, MatchData,
    StateSlot,
};
use whatbot_storage::Store;

static RE_LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^link\s+(\S+)\s+(\S+)\s*$").unwrap());
static RE_UNLINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^unlink\s+(\S+)\s*$").unwrap());

enum Action {
    Link { handle_a: String, handle_b: String },
    Unlink { handle: String },
}

pub struct Admin {
    meta: CommandMeta,
    store: Arc<Store>,
}

impl Admin {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            meta: CommandMeta::core("admin", "admin stuff. link/unlink :alias :alias")
                .require_direct()
                .require_cap(Capability::Admin),
            store,
        }
    }
}

#[async_trait]
impl Command for Admin {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        let text = m.text.trim();

        if let Some(caps) = RE_LINK.captures(text) {
            return Some(MatchData::new(Action::Link {
                handle_a: caps[1].to_string(),
                handle_b: caps[2].to_string(),
            }));
        }
        if let Some(caps) = RE_UNLINK.captures(text) {
            return Some(MatchData::new(Action::Unlink {
                handle: caps[1].to_string(),
            }));
        }
        None
    }

    async fn handle(&self, m: MatchData, ctx: &Context, _state: &mut StateSlot) -> CommandResult {
        let action = match_data!(m => Action);
        match action {
            Action::Link { handle_a, handle_b } => {
                let persons = self.store.persons();

                let acct_a = match persons.account_by_handle(&handle_a).await {
                    Ok(Some(a)) => a,
                    Ok(None) => {
                        return ctx
                            .say(format!("Unknown handle: {handle_a}"))
                            .with_stop(true);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "admin link: account_by_handle failed");
                        return CommandResult::empty();
                    }
                };
                let acct_b = match persons.account_by_handle(&handle_b).await {
                    Ok(Some(a)) => a,
                    Ok(None) => {
                        return ctx
                            .say(format!("Unknown handle: {handle_b}"))
                            .with_stop(true);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "admin link: account_by_handle failed");
                        return CommandResult::empty();
                    }
                };

                // Determine the person to use (prefer an existing one).
                let person = match (acct_a.person_id, acct_b.person_id) {
                    (Some(pa), Some(pb)) if pa == pb => {
                        return ctx
                            .say(format!("{handle_a} and {handle_b} are already linked."))
                            .with_stop(true);
                    }
                    (Some(_pa), Some(_pb)) => {
                        return ctx
                            .say(format!(
                                "{handle_a} and {handle_b} are already linked to different people."
                            ))
                            .with_stop(true);
                    }
                    (Some(_pa), None) => match persons.find_by_display(&handle_a).await {
                        Ok(Some(p)) => p,
                        Ok(None) => match persons.find_or_create(&handle_a).await {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!(error = %e, "admin link: find_or_create failed");
                                return CommandResult::empty();
                            }
                        },
                        Err(e) => {
                            tracing::warn!(error = %e, "admin link: find_by_display failed");
                            return CommandResult::empty();
                        }
                    },
                    (None, Some(_pb)) => match persons.find_by_display(&handle_b).await {
                        Ok(Some(p)) => p,
                        Ok(None) => match persons.find_or_create(&handle_b).await {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!(error = %e, "admin link: find_or_create failed");
                                return CommandResult::empty();
                            }
                        },
                        Err(e) => {
                            tracing::warn!(error = %e, "admin link: find_by_display failed");
                            return CommandResult::empty();
                        }
                    },
                    (None, None) => match persons.find_or_create(&handle_a).await {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::warn!(error = %e, "admin link: find_or_create failed");
                            return CommandResult::empty();
                        }
                    },
                };

                let mut errors = Vec::new();
                if let Err(e) = persons.link_account(acct_a.id, person.id).await {
                    errors.push(format!("{handle_a}: {e}"));
                }
                if let Err(e) = persons.link_account(acct_b.id, person.id).await {
                    errors.push(format!("{handle_b}: {e}"));
                }
                if !errors.is_empty() {
                    tracing::warn!(errors = ?errors, "admin link: partial failure");
                    return ctx.say("Link failed (check logs).").with_stop(true);
                }
                ctx.say(format!(
                    "Linked {handle_a} and {handle_b} as the same person."
                ))
                .with_stop(true)
            }

            Action::Unlink { handle } => {
                let acct = match self.store.persons().account_by_handle(&handle).await {
                    Ok(Some(a)) => a,
                    Ok(None) => {
                        return ctx.say(format!("Unknown handle: {handle}")).with_stop(true);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "admin unlink: account_by_handle failed");
                        return CommandResult::empty();
                    }
                };

                if acct.person_id.is_none() {
                    return ctx
                        .say(format!("{handle} is not linked to anyone."))
                        .with_stop(true);
                }

                if let Err(e) = self.store.persons().unlink_account(acct.id).await {
                    tracing::warn!(error = %e, "admin unlink failed");
                    return CommandResult::empty();
                }

                ctx.say(format!("Unlinked {handle}.")).with_stop(true)
            }
        }
    }
}
