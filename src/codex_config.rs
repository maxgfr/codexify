use crate::fsutil::{atomic_write, read_or_empty};
use crate::paths::Paths;
use crate::state::OriginalToml;
use anyhow::{Context, Result};
use toml_edit::{DocumentMut, Item, Value};

pub fn load(paths: &Paths) -> Result<DocumentMut> {
    let raw = read_or_empty(&paths.config())?;
    if raw.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    raw.parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", paths.config().display()))
}

pub fn save(paths: &Paths, doc: &DocumentMut) -> Result<()> {
    atomic_write(&paths.config(), doc.to_string().as_bytes(), &paths.state)
}

pub fn capture(item: Option<&Item>, original: &mut OriginalToml) {
    if original.captured {
        return;
    }
    original.captured = true;
    original.value = item.map(ToString::to_string);
}

pub fn restore_key(doc: &mut DocumentMut, key: &str, original: &mut OriginalToml) -> Result<()> {
    if !original.captured {
        return Ok(());
    }
    match original.value.take() {
        Some(raw) => {
            let wrapper = format!("value ={raw}\n");
            let parsed = wrapper.parse::<DocumentMut>()?;
            doc[key] = parsed["value"].clone();
        }
        None => {
            doc.remove(key);
        }
    }
    original.captured = false;
    Ok(())
}

pub fn restore_table_key(
    doc: &mut DocumentMut,
    table: &str,
    key: &str,
    original: &mut OriginalToml,
) -> Result<()> {
    if !original.captured {
        return Ok(());
    }
    if let Some(raw) = original.value.take() {
        let wrapper = format!("value ={raw}\n");
        let parsed = wrapper.parse::<DocumentMut>()?;
        doc[table][key] = parsed["value"].clone();
    } else if let Some(target) = doc.get_mut(table).and_then(Item::as_table_like_mut) {
        target.remove(key);
    }
    original.captured = false;
    Ok(())
}

pub fn string_item(value: &str) -> Item {
    Item::Value(Value::from(value))
}
