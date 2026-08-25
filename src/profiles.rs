use crate::codex_config;
use crate::fsutil::{atomic_write, read_or_empty};
use crate::paths::Paths;
use crate::state::ManagedState;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Profile {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProfileFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl ProfileFile {
    pub fn defaults() -> Self {
        let profiles = BTreeMap::from([
            (
                "balanced".to_owned(),
                Profile {
                    model: "gpt-5.6".to_owned(),
                    effort: Some("medium".to_owned()),
                },
            ),
            (
                "fast".to_owned(),
                Profile {
                    model: "gpt-5.6-luna".to_owned(),
                    effort: Some("medium".to_owned()),
                },
            ),
            (
                "quality".to_owned(),
                Profile {
                    model: "gpt-5.6".to_owned(),
                    effort: Some("xhigh".to_owned()),
                },
            ),
        ]);
        Self { profiles }
    }

    pub fn load(paths: &Paths) -> Result<Self> {
        let raw = read_or_empty(&paths.profiles())?;
        if raw.trim().is_empty() {
            let defaults = Self::defaults();
            defaults.save(paths)?;
            return Ok(defaults);
        }
        toml::from_str(&raw).context("failed to parse profiles.toml")
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let raw = toml::to_string_pretty(self)?;
        atomic_write(&paths.profiles(), raw.as_bytes(), &paths.state)
    }
}

pub fn use_target(paths: &Paths, target: &str, effort: Option<&str>) -> Result<()> {
    validate_name(target, "model or profile")?;
    if let Some(value) = effort {
        validate_effort(value)?;
    }
    let profiles = ProfileFile::load(paths)?;
    let selected = profiles.profiles.get(target);
    let model = selected.map_or(target, |profile| profile.model.as_str());
    let effort = effort.or_else(|| selected.and_then(|profile| profile.effort.as_deref()));

    let mut state = ManagedState::load(paths)?;
    let mut doc = codex_config::load(paths)?;
    codex_config::capture(doc.get("model"), &mut state.model);
    codex_config::capture(doc.get("model_reasoning_effort"), &mut state.effort);
    state.save(paths)?;
    doc["model"] = codex_config::string_item(model);
    if let Some(value) = effort {
        doc["model_reasoning_effort"] = codex_config::string_item(value);
    } else {
        doc.remove("model_reasoning_effort");
    }
    codex_config::save(paths, &doc)?;
    println!(
        "Using {model}{}",
        effort
            .map(|v| format!(" with {v} effort"))
            .unwrap_or_default()
    );
    Ok(())
}

pub fn use_profile(paths: &Paths, name: &str) -> Result<()> {
    let profiles = ProfileFile::load(paths)?;
    if !profiles.profiles.contains_key(name) {
        bail!("unknown profile '{name}'");
    }
    use_target(paths, name, None)
}

pub fn reset(paths: &Paths) -> Result<()> {
    let mut state = ManagedState::load(paths)?;
    let mut doc = codex_config::load(paths)?;
    codex_config::restore_key(&mut doc, "model", &mut state.model)?;
    codex_config::restore_key(&mut doc, "model_reasoning_effort", &mut state.effort)?;
    codex_config::save(paths, &doc)?;
    state.save(paths)?;
    println!("Restored Codex's native model configuration.");
    Ok(())
}

pub fn list(paths: &Paths) -> Result<()> {
    let profiles = ProfileFile::load(paths)?;
    for (name, profile) in profiles.profiles {
        println!(
            "{name}\t{}\t{}",
            profile.model,
            profile.effort.as_deref().unwrap_or("native")
        );
    }
    Ok(())
}

pub fn show(paths: &Paths, name: &str) -> Result<()> {
    let profiles = ProfileFile::load(paths)?;
    let profile = profiles
        .profiles
        .get(name)
        .with_context(|| format!("unknown profile '{name}'"))?;
    println!("name = {name}");
    println!("model = {}", profile.model);
    println!("effort = {}", profile.effort.as_deref().unwrap_or("native"));
    Ok(())
}

pub fn add(paths: &Paths, name: &str, model: &str, effort: Option<&str>) -> Result<()> {
    validate_name(name, "profile")?;
    validate_name(model, "model")?;
    if let Some(value) = effort {
        validate_effort(value)?;
    }
    let mut profiles = ProfileFile::load(paths)?;
    profiles.profiles.insert(
        name.to_owned(),
        Profile {
            model: model.to_owned(),
            effort: effort.map(str::to_owned),
        },
    );
    profiles.save(paths)?;
    println!("Saved profile '{name}'.");
    Ok(())
}

pub fn remove(paths: &Paths, name: &str) -> Result<()> {
    let mut profiles = ProfileFile::load(paths)?;
    if profiles.profiles.remove(name).is_none() {
        bail!("unknown profile '{name}'");
    }
    profiles.save(paths)?;
    println!("Removed profile '{name}'.");
    Ok(())
}

fn validate_name(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("invalid {label} '{value}'");
    }
    Ok(())
}

fn validate_effort(value: &str) -> Result<()> {
    if !matches!(value, "minimal" | "low" | "medium" | "high" | "xhigh") {
        bail!("invalid effort '{value}' (expected minimal, low, medium, high, or xhigh)");
    }
    Ok(())
}
