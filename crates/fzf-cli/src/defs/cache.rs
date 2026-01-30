use crate::defs::index::DefIndex;
use crate::util;
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

pub fn load_or_build() -> Result<DefIndex> {
    if !util::env_is_true("FZF_DEF_DOC_CACHE_ENABLED") {
        return super::index::build_index();
    }

    let cache_dir = util::zsh_cache_dir()?;
    let ts_file = cache_dir.join("fzf-def-doc.timestamp");
    let data_file = cache_dir.join("fzf-def-doc.cache.json");

    let ttl_minutes = util::env_or_default("FZF_DEF_DOC_CACHE_EXPIRE_MINUTES", "10")
        .parse::<i64>()
        .unwrap_or(10)
        .max(0);
    let ttl_seconds = ttl_minutes * 60;

    let now = util::now_epoch_seconds();
    if let Ok(raw) = fs::read_to_string(&ts_file) {
        if let Ok(last) = raw.trim().parse::<i64>() {
            if last > 0 && (now - last) <= ttl_seconds {
                if let Ok(data) = fs::read_to_string(&data_file) {
                    if let Ok(index) = serde_json::from_str::<DefIndex>(&data) {
                        return Ok(index);
                    }
                }
            }
        }
    }

    let index = super::index::build_index()?;
    write_cache(&cache_dir, &ts_file, &data_file, &index, now)?;
    Ok(index)
}

fn write_cache(
    cache_dir: &PathBuf,
    ts_file: &PathBuf,
    data_file: &PathBuf,
    index: &DefIndex,
    now: i64,
) -> Result<()> {
    let _ = fs::create_dir_all(cache_dir);
    let data = serde_json::to_string(index).context("serialize def index")?;
    let _ = fs::write(data_file, data);
    let _ = fs::write(ts_file, now.to_string());
    Ok(())
}
