//! Multi-command integration tests via `BotHarness`. Where the unit
//! tests in each module exercise one command in isolation, these tests
//! verify behaviors that **emerge** from how installed commands
//! interact through the dispatcher: tier short-circuits, silent
//! consumers, ambient retrieval.

use std::sync::Arc;

use whatbot_commands::{Factoid, FactoidListener, FactoidStore, SqlFactoidStore};
use whatbot_core::BotHarness;
use whatbot_test_support::Pg;

async fn fresh_factoid_store() -> Arc<dyn FactoidStore> {
    let pg = Pg::shared().await;
    Arc::new(SqlFactoidStore::new(pg.fresh_store().await))
}

/// FactoidListener at `Last` should fire for a bare known subject when
/// nothing in earlier tiers replied. Smoke that the full assignment +
/// listener loop works through the real dispatcher.
#[tokio::test]
async fn factoid_listener_responds_to_bare_subject() {
    let factoid_store = fresh_factoid_store().await;
    let bot = BotHarness::builder()
        .install(Factoid::new(factoid_store.clone()))
        .install(FactoidListener::new(factoid_store))
        .build()
        .await;

    let _ = bot.say("nichelle", "rust is a systems language").await;
    let r = bot.say("nichelle", "rust").await;
    assert_eq!(r, vec!["rust is a systems language".to_string()]);
}

/// Direct retrieval (`what is X`) at Core short-circuits ambient
/// `FactoidListener` at Last so we don't get a double-reply.
#[tokio::test]
async fn what_is_does_not_double_reply_via_listener() {
    let factoid_store = fresh_factoid_store().await;
    let bot = BotHarness::builder()
        .install(Factoid::new(factoid_store.clone()))
        .install(FactoidListener::new(factoid_store))
        .build()
        .await;

    let _ = bot.say("nichelle", "rust is fast").await;
    let r = bot.say("nichelle", "what is rust").await;
    assert_eq!(r.len(), 1, "exactly one reply, not two: {r:?}");
    assert_eq!(r[0], "rust is fast");
}
