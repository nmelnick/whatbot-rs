# what

## Introduction

This is a chatbot, with lineage tracing back to infobot and the dawn of time
and space itself. This repository represents an experimental Rust rethink of
the original whatbot, a Perl monstrosity that somehow perservered, even with
some questionable design decisions and public/private ambiguity.

From the original repository:

> This bot was written purely as an exercise in futility, to try, desperately,
> to replace the functionality of infobot without driving us insane. Part of
> thatgoal has been accomplished, and so we leave it out there for the world to
> use. Drop us a note if you decide to play with it. Maybe if we hit 1.0, we'll
> actually write a few docs. This is really just a project for fun, so there
> really isn't and hasn't been a rallying cry for more documentation and
> support infrastructure, so, uh, there isn't.

Obviously, it never hit "1.0", whatever that is, and now it's just a fun idea
to check out what this could look like with some rethinking and a fundamental
lack of Perl.

## Goals

- Try to keep some of it conceptually similar, with IO representing the chat
  input/output mechanism, a Command representing some sort of action from user
  input (realtime or otherwise), and a Store representing persisted state.
- A chatbot runtime that lives in multiple contexts simultaneously (Discord
  guilds/channels/DMs, a local Console for development, probably the OG IRC and
  more services later).
- A first-class **Context** model so commands know _where_ and _who_ they are
  talking to, including the difference between a public channel, a DM, and a
  thread. The original whatbot was _super poor_ at this.
- A first-class **Identity** seam: per-service `Account` rows now, and maybe
  look at linking people across contexts later.
- The "x is y" factoid memory that gave this thing something that looked like
  sentience should come over pretty much unchanged.
- Tokio-native async throughout. Each IO is a task; the dispatcher is a task;
  commands are `async fn`. This sorta-kinda existed in Perl Whatbot using
  AnyEvent, this should be designed with that from the start.
- Postgres-only storage via `sqlx` with managed migrations. SQLite was there
  for easy development, but Docker makes that a non-issue.
- A clean `Command` trait so new behavior can be added as a Rust module (or
  later, a separate crate) without runtime plugin loading.

## Repository

The layout is subject to change, but going to try to utilize a workspace and
individual crates to separate concerns, as we're not going to try to shoehorn
Perl-esque object oriented development in this decade. As it stands currently:

```
whatbot/
 - crates/
   - whatbot-core/       Types, Command trait, Dispatcher
   - whatbot-storage/    The sqlx pool, migrations runner, repositories
   - whatbot-io-console/ Stdin/stdout IO, mostly for dev
   - whatbot-io-discord/ Discord IO through the serenity library
   - whatbot-commands/   Built-in commands
   - whatbot/            Binary
 - migrations/           Managed SQL
 - conf/                 Example configuration, probably the runtime config
```
