# Creating a new command

This document builds a small command to a functional state: matching, replying, configuring, persisting per-channel
state, and wiring it into the binary. By the end you'll have a working `Ping` command and know where to look in the
repository for everything else.

If you'd rather read working code first, start with `crates/whatbot-commands/src/echo.rs` — that's the simplest real
command in the tree.

## Operation

A **command** is a Rust type that implements the [`Command`](../crates/whatbot-core/src/command.rs) trait. The runtime
calls two methods on it per inbound event:

**matches** _(sync)_: Decide whether you want to handle this event
**handle** _(async)_: Produce a reply (or not)

If `matches` returns `None`, the dispatcher skips the command entirely and never calls `handle`. If it returns
`Some(MatchData)`, the dispatcher then calls `handle(match_data, ctx, state)`.

Commands run inside the **Dispatcher**, which sits between **IOs** (Discord, Console, whatever) and your code. The IO
emits a `RawEvent` and the dispatcher resolves the speaker's identity, picks a service-specific mention renderer, walks
installed commands by priority, and routes any `Reply`s back to the right IO. This hopefully leaves the **Command** to
focus on consuming Context and Event and producing Reply.

## Priorities and the tier short-circuit

Commands declare a priority tier:

1. `Primary`: always runs, before everything else, and is used for functionality that should always run
2. `Core`: also always runs, exists for internal bot functionality, and has the right to stop
3. `Extension`: most functionality built outside of the core exists here, but is skipped if any earlier tier produces
   output
4. `Last`: a run of last resort, skipped if any earlier tier produces output, and used as last resort functionality

Within a tier, every command's `matches` is queried. If your `handle` returns a non-empty reply (or sets `consumed` to
true), the dispatcher records that the event was handled and skips `Extension`/`Last` entirely. This is how
`FactoidListener` at `Priority::Last` only chimes in when nothing more specific responded.

## Building Hello world: the `Ping` command

Create `crates/whatbot-commands/src/ping.rs`:

```rust
use async_trait::async_trait;
use whatbot_core::{
    match_data, Command, CommandMeta, CommandResult, Context, Event, MatchData, StateSlot,
};

pub struct Ping {
    meta: CommandMeta,
}

impl Ping {
    pub fn new() -> Self {
        Self {
            meta: CommandMeta::core("ping", "ping — replies with pong"),
        }
    }
}

impl Default for Ping {
    fn default() -> Self {
        Self::new()
    }
}

/// What `matches` returns. The dispatcher sees an opaque `MatchData`;
/// `handle` recovers the type with `match_data!`.
struct PingMatch;

#[async_trait]
impl Command for Ping {
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    fn matches(&self, evt: &Event, _ctx: &Context) -> Option<MatchData> {
        let Event::Message(m) = evt else { return None };
        if m.text.trim().eq_ignore_ascii_case("ping") {
            Some(MatchData::new(PingMatch))
        } else {
            None
        }
    }

    async fn handle(
        &self,
        m: MatchData,
        ctx: &Context,
        _state: &mut StateSlot,
    ) -> CommandResult {
        let _ = match_data!(m => PingMatch);
        ctx.say("pong")
    }
}
```

### What

- `CommandMeta::core("ping", ...)` names the command under a priority. That name is used when initially loading it, for
  help listings, and if the command is configurable, provides the heading in the config file (see below for more info).
  For a different priority, ::primary, ::extension, and ::last_resort are available. After the constructor, one can
  chain `.require_direct()` if the bot must be addressed by name, and/or `.require_cap(Capability::Admin)` for commands
  requiring some sort of privilege.
- `matches` returns `Option<MatchData>`. The contents of `MatchData` are opaque to the dispatcher, it's just routing the
  contents back, so pick whatever type makes sense. In this case, we use an empty marker because there's nothing to pass
  through.
- `match_data!(m => PingMatch)` unwraps the boxed `Any` back into your type. On a type mismatch (which should't happen
  unless `matches` and `handle` somehow differ), it will return `CommandResult::empty()`.
- `ctx.say("pong")` is shorthand for `CommandResult::reply(ctx.reply_here("pong"))`, which accounts for most of the
  replyish methods a command will make. The reply is addressed to the same channel the incoming message arrived through.
  One can chain `.with_stop(true)` and/or `.with_consumed(true)` to stop later commands from firing.

## Observing messages with the `Monitor` trait

If your command needs to observe every message without replying, like recording activity, tracking statistics, stalking,
implement [`Monitor`](../crates/whatbot-core/src/monitor.rs) instead of`Command`. It removes all the `matches`/`handle`/
`MatchData` boilerplate:

```rust
use async_trait::async_trait;
use whatbot_core::{Context, Monitor};
use whatbot_storage::Store;
use std::sync::Arc;

pub struct ActivityTracker {
    store: Arc<Store>,
}

impl ActivityTracker {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Monitor for ActivityTracker {
    fn name(&self) -> &'static str {
        "activity_tracker"
    }

    async fn observe(&self, ctx: &Context, text: &str) {
        self.store
            .your_repo()
            .record(&ctx.author.display, text)
            .await
            .ok();
    }
}
```

### What

In this case, `observe` is called with the trimmed, non-empty message text. The `Monitor` wrapper runs at
`Priority::Primary`, always fires, and never produces replies. Register it with `registry.install_monitor(...)` rather
than `install_command(...)`.

## Register it

`crates/whatbot/src/main.rs` has one block where commands are installed.

For a normal command, add a line:

```rust
install_command(&mut registry, "ping", &cfg.commands, |_| Ok(Ping::new()))?;
```

For a monitor:

```rust
registry.install_monitor(ActivityTracker::new(store.clone()));
```

Export the type from `crates/whatbot-commands/src/lib.rs`:

```rust
pub mod ping;
pub use ping::Ping;
```

That's it. Run `cargo run` and type `ping` in the console.

## Configuring the world

A configuration can be passed into a command through the main config file. For our case, let's say we want to customize
the reply sent to the caller. First, let's define our config structure in a lightly refactored Ping:

```rust
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PingConfig {
    #[serde(default = "default_reply")]
    pub reply: String,
}

impl Default for PingConfig {
    fn default() -> Self {
        Self { reply: default_reply() }
    }
}

fn default_reply() -> String {
    "pong".to_string()
}

pub struct Ping {
    meta: CommandMeta,
    config: PingConfig,
}

impl Ping {
    pub fn new() -> Self {
        Self::with_config(PingConfig::default())
    }

    pub fn with_config(config: PingConfig) -> Self {
        Self {
            meta: CommandMeta::core("ping", "ping — replies with a configurable string"),
            config,
        }
    }
}
```

In `handle`, use `self.config.reply` instead of the string we had before.

In `main.rs`, send the config through:

```rust
install_command(&mut registry, "ping", &cfg.commands, |c| {
    Ok(Ping::with_config(c.typed()?))
})?;
```

Now, in our config file:

```toml
[commands.ping]
reply = "ack"
```

Operators can disable any command by setting `enabled` to `false`:

```toml
[commands.ping]
enabled = false
```

The runtime will skip installing the command.

## Per-context state

If your command needs to remember things across messages, such as a counter per channel, a toggle, who spoke last, use
the per-context state slot. State is keyed by `(service, channel)`, so toggling in `#general` doesn't leak into
`#random`.

Define a type with `Default`:

```rust
#[derive(Default, Debug)]
struct PingScratch {
    count: u32,
}
```

In `handle`, read or modify it:

```rust
async fn handle(
    &self,
    m: MatchData,
    ctx: &Context,
    state: &mut StateSlot,
) -> CommandResult {
    let _ = match_data!(m => PingMatch);
    let count = state
        .with::<PingScratch, _, _>(|s| {
            s.count += 1;
            s.count
        })
        .await;
    ctx.say(format!("pong ({count})"))
}
```

`StateSlot::with::<T, _, _>(f)` looks up your typed slot (defaulting on first access), calls `f`, and returns whatever
`f` returns. Different commands using the same `(service, channel)` slot shouldn't collide, as each type gets its own
slot keyed by `TypeId`.

## Replies and routing

A few `Context` helpers:

- `ctx.say(text)`: `CommandResult` with one reply in the inbound channel
- `ctx.reply_here(text)`: `Reply`, when you build a multi-reply `CommandResult` by hand
- `ctx.reply_direct(text)`: DM to the author, even if they asked in a public channel
- `ctx.mention(&account)`: Service-specific mention token (`<@123>` on Discord or display name on Console)
- `ctx.has(&capability)`: True if `ctx.author` has the capability
- `ctx.addressed_to_bot`: True if the user named the bot or it's a DM
- `ctx.author.matches_handle("x")`: Case-insensitive handle compare

To be nice to the runtime, if your command shouldn't act on messages not directed act the bot, chain `require_direct` on
the `CommandMeta` rather than checking `addressed_to_bot` inside `handle`. The dispatcher checks `require_direct` before
ever calling `matches`, so unrelated chat doesn't even enter a matching flow.

## Getting (or, actually, reponding with) results

`CommandResult` carries three things:

- `replies: Vec<Reply>`: output to the user(s)
- `stop: bool`: short-circuits this tier (other commands don't run after you)
- `consumed: bool`: counts as "handled", even if you emitted no reply

The constructors:

- `CommandResult::empty()`: no reply, no stop, not consumed
- `CommandResult::reply(reply)`: one reply
- `CommandResult::stop()`: empty response, but stop
- `CommandResult::handled_silently()`: empty response, stop, set consumed

## Persistence

For state that must survive restarts, add a repository to `whatbot-storage` and use `Arc<Store>` directly in your
command. Whatbot currently uses `sqlx` as the interface to PostgreSQL.

### Add a migration

Create a new file in `/migrations/`, e.g. `20260602000001_ping_history.sql`:

```sql
CREATE TABLE ping_history (
    id          BIGSERIAL PRIMARY KEY,
    handle      TEXT NOT NULL,
    seen_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Migrations run automatically on startup via `store.migrate()`.

### Add a repo in `whatbot-storage`

Create `crates/whatbot-storage/src/ping_history.rs`:

```rust
use sqlx::{PgPool, Row};
use crate::error::StorageError;

pub struct PingHistoryRepo<'a> {
    pool: &'a PgPool,
}

impl<'a> PingHistoryRepo<'a> {
    pub fn new(pool: &'a PgPool) -> Self { Self { pool } }

    pub async fn record(&self, handle: &str) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO ping_history (handle) VALUES ($1)",
        )
        .bind(handle)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
```

Declare it in `crates/whatbot-storage/src/lib.rs`:

```rust
pub mod ping_history;
```

And add a `ping_history()` accessor to `Store` in `crates/whatbot-storage/src/store.rs`:

```rust
pub fn ping_history(&self) -> PingHistoryRepo<'_> {
    PingHistoryRepo::new(&self.pool)
}
```

### Use it

Hold `Arc<Store>` in your struct and call the repo:

```rust
pub struct Ping {
    meta: CommandMeta,
    store: Arc<Store>,
}

impl Ping {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            meta: CommandMeta::core("ping", "ping — replies with pong"),
            store,
        }
    }
}

// In handle():
self.store.ping_history().record(&ctx.author.display).await.ok();
```

### Wire up in `main.rs`

```rust
let ping_store = store.clone();
// ...
install_command(&mut registry, "ping", &cfg.commands, |_| {
    Ok(Ping::new(ping_store.clone()))
})?;
```
