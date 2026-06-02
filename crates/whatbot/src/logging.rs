//! Server log + transcript wiring.

use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use whatbot_core::transcript::{Direction, TranscriptEntry};
use whatbot_core::Visibility;

use crate::config::TranscriptConfig;

pub fn init_tracing() -> Option<WorkerGuard> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stdout_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);
    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .init();
    None
}

pub fn spawn_transcript_writer(
    cfg: &TranscriptConfig,
    mut rx: mpsc::Receiver<TranscriptEntry>,
) -> Option<tokio::task::JoinHandle<()>> {
    let path = cfg.file.clone()?;
    Some(tokio::spawn(async move {
        let file = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
        {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(?e, path = %path, "couldn't open transcript file");
                return;
            }
        };
        let mut writer = tokio::io::BufWriter::new(file);
        while let Some(entry) = rx.recv().await {
            let line = format_entry(&entry);
            if let Err(e) = writer.write_all(line.as_bytes()).await {
                tracing::warn!(?e, "transcript write failed");
                break;
            }
            // Flush each line so tail -f works.
            if let Err(e) = writer.flush().await {
                tracing::warn!(?e, "transcript flush failed");
                break;
            }
        }
        let _ = writer.flush().await;
    }))
}

fn format_entry(e: &TranscriptEntry) -> String {
    let arrow = match e.direction {
        Direction::Incoming => "<",
        Direction::Outgoing => ">",
    };
    let visibility = match e.visibility {
        Visibility::Public => "",
        Visibility::Private => " (dm)",
        Visibility::Thread { .. } => " (thread)",
    };
    format!(
        "{ts} [{service}/{channel}{visibility}] {arrow}{speaker}> {text}\n",
        ts = e.ts.format("%Y-%m-%dT%H:%M:%SZ"),
        service = e.service.as_str(),
        channel = e.channel.as_str(),
        speaker = e.speaker,
        text = e.text,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use whatbot_core::transcript::{Direction, TranscriptEntry};
    use whatbot_core::Visibility;
    use whatbot_core::{ChannelId, ServiceId};

    fn entry(direction: Direction, speaker: &str, text: &str, vis: Visibility) -> TranscriptEntry {
        TranscriptEntry {
            ts: Utc::now(),
            direction,
            service: ServiceId::new("console"),
            channel: ChannelId::new("general"),
            visibility: vis,
            speaker: speaker.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn incoming_public_line() {
        let line = format_entry(&entry(
            Direction::Incoming,
            "nichelle",
            "rust is fast",
            Visibility::Public,
        ));
        assert!(line.contains("[console/general]"));
        assert!(line.contains("<nichelle> rust is fast"));
        assert!(line.ends_with('\n'));
    }

    #[test]
    fn outgoing_dm_marked() {
        let line = format_entry(&entry(
            Direction::Outgoing,
            "whatbot",
            "hi nichelle",
            Visibility::Private,
        ));
        assert!(line.contains("(dm)"));
        assert!(line.contains(">whatbot> hi nichelle"));
    }
}
