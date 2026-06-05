# Migrating from Perl whatbot

The `whatbot-migrate` binary imports a whatbot SQLite database into the new Postgres schema.

Right now, it just supports:

* factoid
* karma

```bash
cargo run -p whatbot-migrate -- \
    --sqlite /path/to/old/whatbot.db \
    --postgres "postgres://whatbot:whatbot@localhost/whatbot"
```

## Flags

`--sqlite PATH`: Path to the legacy SQLite file.

`--postgres URL`: Postgres connection string, in the form `postgres://{username}:{password}@{host}:{port}/{database}`
