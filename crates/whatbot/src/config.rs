//! TOML configuration for the whatbot binary.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub bot: BotConfig,
    pub database: Option<DatabaseConfig>,
    #[serde(default)]
    pub transcript: TranscriptConfig,
    #[serde(default)]
    pub io: Vec<IoConfig>,
    #[serde(default)]
    pub commands: HashMap<String, toml::Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TranscriptConfig {
    pub file: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BotConfig {
    #[serde(default = "default_bot_name")]
    pub name: String,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            name: default_bot_name(),
        }
    }
}

fn default_bot_name() -> String {
    "whatbot".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum IoConfig {
    Console(ConsoleIoConfig),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsoleIoConfig {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default = "default_console_user")]
    pub user: String,
}

fn default_console_user() -> String {
    "user".to_string()
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", path.display()))?;
        Ok(cfg)
    }
}

pub struct CommandConfig<'a> {
    pub enabled: bool,
    raw: Option<&'a toml::Value>,
}

impl<'a> CommandConfig<'a> {
    pub fn for_name(name: &str, commands: &'a HashMap<String, toml::Value>) -> Self {
        let raw = commands.get(name);
        let enabled = raw
            .and_then(|v| v.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        Self { enabled, raw }
    }

    #[allow(dead_code)]
    pub fn typed<T>(&self) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned + Default,
    {
        match self.raw {
            Some(v) => Ok(v.clone().try_into()?),
            None => Ok(T::default()),
        }
    }

    #[allow(dead_code)]
    pub fn raw(&self) -> Option<&toml::Value> {
        self.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_console() {
        let toml = r#"
            [[io]]
            kind = "console"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.bot.name, "whatbot");
        assert!(cfg.database.is_none());
        assert_eq!(cfg.io.len(), 1);
        match &cfg.io[0] {
            IoConfig::Console(c) => assert_eq!(c.user, "user"),
            other => panic!("unexpected io: {other:?}"),
        }
    }

    #[test]
    fn command_config_defaults_to_enabled_when_absent() {
        let commands = HashMap::new();
        let cfg = CommandConfig::for_name("factoid", &commands);
        assert!(cfg.enabled);
        assert!(cfg.raw().is_none());
    }

    #[test]
    fn command_config_honors_enabled_flag() {
        let toml = r#"
            [commands.karma]
            enabled = false
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let karma = CommandConfig::for_name("karma", &cfg.commands);
        assert!(!karma.enabled);
        let factoid = CommandConfig::for_name("factoid", &cfg.commands);
        assert!(factoid.enabled);
    }

    #[test]
    fn command_config_typed_returns_default_when_section_absent() {
        #[derive(serde::Deserialize, Default, PartialEq, Debug)]
        struct Demo {
            #[serde(default)]
            knob: u32,
        }

        let commands = HashMap::new();
        let cfg = CommandConfig::for_name("demo", &commands);
        let parsed: Demo = cfg.typed().unwrap();
        assert_eq!(parsed, Demo::default());
    }

    #[test]
    fn command_config_typed_deserializes_section() {
        #[derive(serde::Deserialize, Default, PartialEq, Debug)]
        struct Demo {
            knob: u32,
            name: String,
        }

        let toml = r#"
            [commands.demo]
            knob = 7
            name = "hi"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let parsed: Demo = CommandConfig::for_name("demo", &cfg.commands)
            .typed()
            .unwrap();
        assert_eq!(parsed.knob, 7);
        assert_eq!(parsed.name, "hi");
    }

    #[test]
    fn command_config_typed_ignores_enabled_field() {
        #[derive(serde::Deserialize, Default, PartialEq, Debug)]
        #[serde(deny_unknown_fields)]
        struct Strict {
            knob: u32,
        }

        let toml = r#"
            [commands.demo]
            enabled = true
            knob = 9
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let typed: Result<Strict, _> = CommandConfig::for_name("demo", &cfg.commands).typed();
        assert!(
            typed.is_err(),
            "deny_unknown_fields + enabled = expected error (don't use deny_unknown_fields)"
        );
    }
}
