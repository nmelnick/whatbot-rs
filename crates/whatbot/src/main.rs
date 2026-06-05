//! whatbot binary entry point.

mod config;
mod logging;

use std::path::PathBuf;
use std::sync::Arc;

use whatbot_commands::{Awareness, Echo, Excuse, Factoid, FactoidListener, Help, Karma, Seen, SeenRecorder};
use whatbot_core::dispatcher::IdentityResolver;
use whatbot_core::{Dispatcher, Io, Registry, TranscriptHandle};
use whatbot_io_console::ConsoleIo;
use whatbot_io_discord::{DiscordConfig, DiscordIo};
use whatbot_storage::Store;

use crate::config::{CommandConfig, Config, IoConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to load config: {e}");
            return Err(e);
        }
    };

    let _log_guard = logging::init_tracing();

    tracing::info!(bot = %cfg.bot.name, "starting whatbot");

    let db = cfg.database.as_ref().ok_or_else(|| {
        anyhow::anyhow!("[database] section is required; set database.url in the config")
    })?;
    let store = Arc::new(Store::connect(&db.url).await?);
    store.migrate().await?;
    tracing::info!("connected to postgres and migrated");
    let seen_store = store.clone();
    let factoid_store = store.clone();
    let karma_store = store.clone();
    let identity: Arc<dyn IdentityResolver> = store;

    let mut registry = Registry::new();
    registry.install_monitor(SeenRecorder::new(seen_store.clone()));
    tracing::info!(command = "seen_recorder", "installed");
    install_command(&mut registry, "awareness", &cfg.commands, |_| Ok(Awareness::new()))?;
    install_command(&mut registry, "echo", &cfg.commands, |_| Ok(Echo::new()))?;
    install_command(&mut registry, "excuse", &cfg.commands, |_| Ok(Excuse::new()))?;
    install_command(&mut registry, "factoid", &cfg.commands, |_| {
        Ok(Factoid::new(factoid_store.clone()))
    })?;
    install_command(&mut registry, "karma", &cfg.commands, |_| {
        Ok(Karma::new(karma_store.clone()))
    })?;
    // Catch all factoid retrieval lives at Priority::Last; it only fires when
    // no other command produced output.
    install_command(&mut registry, "factoid_listener", &cfg.commands, |_| {
        Ok(FactoidListener::new(factoid_store.clone()))
    })?;
    install_command(&mut registry, "seen", &cfg.commands, |_| {
        Ok(Seen::new(seen_store.clone()))
    })?;
    // Help snapshots the registry at construction time, so install it last
    let help_cfg = CommandConfig::for_name("help", &cfg.commands);
    if help_cfg.enabled {
        registry.install(Arc::new(Help::from_registry(&registry)));
        tracing::info!(command = "help", "installed");
    } else {
        tracing::info!(command = "help", "disabled in config");
    }

    let mut dispatcher = Dispatcher::new(registry, identity, 64);

    // Transcription only happens if a file path is configured.
    let mut transcript_task = None;
    if cfg.transcript.file.is_some() {
        let (handle, rx) = TranscriptHandle::channel(256);
        dispatcher.set_transcript(handle);
        transcript_task = logging::spawn_transcript_writer(&cfg.transcript, rx);
        tracing::info!(file = %cfg.transcript.file.as_deref().unwrap_or(""), "transcript enabled");
    }

    // Build a uniform list of IOs from config. New IO implements
    // `whatbot_core::Io` and adding a config.
    let ios: Vec<Box<dyn Io>> = cfg.io.iter().map(build_io).collect::<anyhow::Result<_>>()?;

    let mut io_tasks = Vec::new();
    for io in ios {
        let service_label = io.service_id().as_str().to_string();
        let renderer = io.mention_renderer();
        let handle = io.start(dispatcher.inbound_sender()).await?;
        dispatcher.register_mention_renderer(handle.service.clone(), renderer);
        dispatcher.register_outbound(handle.service, handle.outbound);
        if let Some(task) = handle.task {
            io_tasks.push(task);
        }
        tracing::info!(service = %service_label, "io enabled");
    }

    dispatcher.run().await?;
    for task in io_tasks {
        task.abort();
    }
    if let Some(t) = transcript_task {
        // If we don't wait, the handle is dropped, which sucks
        let _ = t.await;
    }
    Ok(())
}

/// Install a command into the registry if `[commands.<name>]` didn't disable
/// it.
fn install_command<C: whatbot_core::Command + 'static>(
    registry: &mut Registry,
    name: &str,
    commands: &std::collections::HashMap<String, toml::Value>,
    build: impl FnOnce(&CommandConfig<'_>) -> anyhow::Result<C>,
) -> anyhow::Result<()> {
    let cfg = CommandConfig::for_name(name, commands);
    if !cfg.enabled {
        tracing::info!(command = name, "disabled in config");
        return Ok(());
    }
    let cmd =
        build(&cfg).map_err(|e| anyhow::anyhow!("failed to construct command {name}: {e}"))?;
    registry.install(Arc::new(cmd));
    tracing::info!(command = name, "installed");
    Ok(())
}

// This is going to require some work once more IO exists
fn build_io(cfg: &IoConfig) -> anyhow::Result<Box<dyn Io>> {
    Ok(match cfg {
        IoConfig::Console(c) => {
            let mut io = ConsoleIo::new("whatbot", &c.user);
            if let Some(id) = &c.id {
                io = io.with_service(id);
            }
            Box::new(io)
        }
        IoConfig::Discord(d) => {
            let mut dc = DiscordConfig::new(d.token.clone());
            if let Some(id) = &d.id {
                dc.service_id = whatbot_core::ServiceId::new(id);
            }
            if let Some(p) = &d.addressed_prefix {
                dc = dc.with_addressed_prefix(p.clone());
            }
            Box::new(DiscordIo::new(dc))
        }
    })
}

fn load_config() -> anyhow::Result<Config> {
    // Search order: $WHATBOT_CONFIG, ./conf/whatbot.toml
    if let Ok(env_path) = std::env::var("WHATBOT_CONFIG") {
        return Config::load(&PathBuf::from(env_path));
    }
    let default_path = PathBuf::from("conf/whatbot.toml");
    if default_path.exists() {
        return Config::load(&default_path);
    }
    anyhow::bail!("no config file found")
}
