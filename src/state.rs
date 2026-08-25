use crate::fsutil::{atomic_write, read_or_empty};
use crate::paths::Paths;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ManagedState {
    #[serde(default)]
    pub model: OriginalToml,
    #[serde(default)]
    pub effort: OriginalToml,
    #[serde(default)]
    pub notify: OriginalToml,
    #[serde(default)]
    pub tui_notifications: OriginalToml,
    #[serde(default)]
    pub tui_notification_method: OriginalToml,
    #[serde(default)]
    pub tui_notification_condition: OriginalToml,
    #[serde(default)]
    pub tui_table_existed: bool,
    #[serde(default)]
    pub notifications_enabled: bool,
    #[serde(default)]
    pub caffeine_mode: Option<String>,
    #[serde(default)]
    pub backup_auto: bool,
    #[serde(default)]
    pub backup_hooks: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OriginalToml {
    pub captured: bool,
    pub value: Option<String>,
}

impl ManagedState {
    pub fn load(paths: &Paths) -> Result<Self> {
        let raw = read_or_empty(&paths.state_file())?;
        if raw.is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(&raw).context("failed to parse Codexify state")
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        atomic_write(&paths.state_file(), &bytes, &paths.state)
    }
}

#[cfg(test)]
mod tests {
    use super::ManagedState;

    #[test]
    fn state_from_before_tui_backend_management_remains_readable() {
        let state: ManagedState = serde_json::from_str(
            r#"{
                "tui_notifications": {"captured": true, "value": "[\"agent-turn-complete\"]"},
                "notifications_enabled": true
            }"#,
        )
        .unwrap();

        assert!(!state.tui_notification_method.captured);
        assert!(!state.tui_notification_condition.captured);
        assert!(state.notifications_enabled);
    }
}
