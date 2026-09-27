use crate::resource_limits::ResourceLimits;
use anyhow::{Context, Result};
use std::sync::Mutex;

static CACHE_WRITE: Mutex<()> = Mutex::new(());

const CATALOG_CACHE_DIR: &str = "ux0:data/green-vita-540-test/cache/catalog-v1";

pub(super) fn provider_path(namespace: &str, filename: &str) -> String {
    format!(
        "{CATALOG_CACHE_DIR}/{}/{}",
        safe_component(namespace),
        safe_filename(filename)
    )
}

pub(super) fn game_path(namespace: &str, locale: &str, game_id: &str, filename: &str) -> String {
    format!(
        "{CATALOG_CACHE_DIR}/{}/{}/{}/{}",
        safe_component(namespace),
        safe_component(locale),
        safe_component(game_id),
        safe_filename(filename)
    )
}

pub(super) fn read(path: &str) -> Option<Vec<u8>> {
    crate::fs_utils::read_bounded(path, ResourceLimits::MAX.metadata_bytes).ok()
}

pub(super) fn write(path: &str, bytes: impl AsRef<[u8]>) -> Result<()> {
    let _lock = CACHE_WRITE
        .lock()
        .map_err(|_| anyhow::anyhow!("cache write unavailable"))?;
    let bytes = bytes.as_ref();
    admit(
        std::path::Path::new(CATALOG_CACHE_DIR),
        bytes.len(),
        ResourceLimits::default(),
    )?;
    let directory = path
        .rsplit_once('/')
        .map(|(directory, _)| directory)
        .context("catalog cache path has no parent")?;
    std::fs::create_dir_all(directory)
        .with_context(|| format!("failed to create cache directory {directory}"))?;
    crate::fs_utils::write_file_truncating(path, bytes)
}

pub(super) fn clear() -> Result<()> {
    match std::fs::remove_dir_all(CATALOG_CACHE_DIR) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove cache directory {CATALOG_CACHE_DIR}")),
    }
}

fn safe_filename(value: &str) -> String {
    if value.is_empty() || matches!(value, "." | "..") || value.contains(['/', '\\', ':']) {
        safe_component(value)
    } else {
        value.to_owned()
    }
}

fn safe_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "_".to_owned()
    } else {
        sanitized
    }
}

fn admit(root: &std::path::Path, incoming: usize, limits: ResourceLimits) -> Result<()> {
    let limits = limits.validate()?;
    anyhow::ensure!(incoming <= limits.metadata_bytes, "cache item too large");
    let mut items = 0usize;
    let mut bytes = incoming as u64;
    let mut directories = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(directory) = directories.pop() {
        visited += 1;
        anyhow::ensure!(
            visited <= limits.cache_items * 4,
            "cache directory limit reached"
        );
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => anyhow::bail!("cache cannot be inspected"),
        };
        for entry in entries {
            let entry = entry.context("cache cannot be inspected")?;
            let kind = entry.file_type().context("cache cannot be inspected")?;
            if kind.is_dir() {
                anyhow::ensure!(
                    directories.len() < limits.cache_items,
                    "cache directory limit reached"
                );
                directories.push(entry.path());
            } else if kind.is_file() {
                items += 1;
                bytes = bytes
                    .checked_add(entry.metadata()?.len())
                    .context("cache size overflow")?;
            } else {
                anyhow::bail!("unsupported cache entry");
            }
            // Include the replacement's temporary file in the budget. Conservatively
            // refuse at the cap instead of deleting a user's existing cache entries.
            anyhow::ensure!(
                items < limits.cache_items && bytes <= limits.cache_bytes,
                "cache budget reached"
            );
        }
    }
    anyhow::ensure!(bytes <= limits.cache_bytes, "cache budget reached");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_admission_counts_files_and_bytes_including_pending_write() {
        let root =
            std::env::temp_dir().join(format!("greenvita-cache-limit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("existing"), [0; 8]).unwrap();
        let limits = ResourceLimits {
            cache_items: 2,
            cache_bytes: 10,
            ..Default::default()
        };
        assert!(admit(&root, 2, limits).is_ok());
        assert!(admit(&root, 3, limits).is_err());
        assert!(
            admit(
                &root,
                1,
                ResourceLimits {
                    cache_items: 1,
                    ..limits
                }
            )
            .is_err()
        );
    }
}
