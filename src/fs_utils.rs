//! Persistence helpers. Never truncate or remove the destination before replacement.
use anyhow::{Context, Result, ensure};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        // Only our newly created temporary file; never the previous destination.
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn read_bounded(path: impl AsRef<Path>, limit: usize) -> Result<Vec<u8>> {
    let probe = u64::try_from(limit)
        .ok()
        .and_then(|n| n.checked_add(1))
        .context("invalid saved-data byte limit")?;
    let file = std::fs::File::open(path).context("could not open saved data")?;
    ensure!(
        file.metadata()?.len() <= limit as u64,
        "saved data exceeds size limit"
    );
    let mut bytes = Vec::new();
    file.take(probe)
        .read_to_end(&mut bytes)
        .context("could not read saved data")?;
    ensure!(bytes.len() <= limit, "saved data exceeds size limit");
    Ok(bytes)
}

pub fn write_file_truncating(path: &str, data: impl AsRef<[u8]>) -> Result<()> {
    atomic_write(Path::new(path), |file| file.write_all(data.as_ref()))
}

fn atomic_write(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .context("saved data has no parent directory")?;
    let name = path
        .file_name()
        .context("saved data has no file name")?
        .to_string_lossy();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp_path)
        .context("could not create temporary saved data")?;
    let _temporary = Temporary(temp_path.clone());
    write(&mut file).context("could not write saved data; previous file preserved")?;
    file.flush()
        .context("could not flush saved data; previous file preserved")?;
    file.sync_all()
        .context("could not sync saved data; previous file preserved")?;
    drop(file);
    // On platforms that cannot replace an existing file atomically, return an
    // error. Removing the old file as a fallback would violate data integrity.
    std::fs::rename(&temp_path, path)
        .context("could not replace saved data; previous file preserved")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory() -> PathBuf {
        // PIDs can repeat across harness invocations. Never reuse a previous
        // fixture or remove its contents just to make a repeated test pass.
        for _ in 0..128 {
            let path = std::env::temp_dir().join(format!(
                "greenvita-save-{}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not create isolated test directory: {error}"),
            }
        }
        panic!("could not allocate an unused test directory after 128 attempts")
    }
    #[test]
    fn interrupted_write_preserves_last_good_file() {
        let path = directory().join("settings.json");
        std::fs::write(&path, b"old-valid-settings").unwrap();
        let result = atomic_write(&path, |file| {
            file.write_all(b"partial")?;
            Err(std::io::Error::other("injected write failure"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old-valid-settings");
    }
    #[test]
    fn replacement_truncates_without_destroying_old_data_first() {
        let path = directory().join("settings.json");
        std::fs::write(&path, b"long old file").unwrap();
        write_file_truncating(path.to_str().unwrap(), b"{}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        assert!(read_bounded(&path, 1).is_err());
    }
    #[test]
    fn rename_failure_preserves_destination() {
        let path = directory().join("occupied");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("old"), b"keep").unwrap();
        assert!(atomic_write(&path, |file| file.write_all(b"new")).is_err());
        assert_eq!(std::fs::read(path.join("old")).unwrap(), b"keep");
    }
}
