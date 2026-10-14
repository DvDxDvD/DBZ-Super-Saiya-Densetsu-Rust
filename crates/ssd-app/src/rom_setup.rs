use anyhow::Context;
use std::path::Path;

pub enum RomSetup {
    Ready(Vec<u8>),
    Missing,
}

// Not derived: a derived impl would print the whole ROM image into panics and logs.
impl std::fmt::Debug for RomSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready(bytes) => write!(f, "Ready({} bytes)", bytes.len()),
            Self::Missing => f.write_str("Missing"),
        }
    }
}

/// Turns the launch arguments into a ready ROM image.
///
/// With `--rom` the image is identified, normalised and written to the cache (via a temp file and a
/// rename, so an interrupted run never leaves a half-written cache). Without it the cache is read
/// and re-validated.
///
/// # Errors
///
/// Fails if a given file cannot be read, the ROM is not one of the accepted images, or the cached
/// ROM is unreadable or damaged.
pub fn resolve_rom(rom_arg: Option<&Path>, ips_arg: Option<&Path>) -> anyhow::Result<RomSetup> {
    let cache = crate::paths::cached_rom();
    if let Some(rom_path) = rom_arg {
        let input = std::fs::read(rom_path)
            .with_context(|| format!("reading ROM {}", rom_path.display()))?;
        let ips = ips_arg
            .map(|path| {
                std::fs::read(path).with_context(|| format!("reading patch {}", path.display()))
            })
            .transpose()?;
        let rom = ssd_rom::normalize(&input, ips.as_deref())?;
        std::fs::create_dir_all(cache.parent().expect("cache path has a parent"))?;
        let tmp = cache.with_extension("tmp");
        std::fs::write(&tmp, &rom.bytes)?;
        std::fs::rename(&tmp, &cache)?;
        log::info!("ROM accepted ({:?}) and cached at {}", rom.source, cache.display());
        return Ok(RomSetup::Ready(rom.bytes));
    }
    match std::fs::read(&cache) {
        Ok(bytes) if ssd_rom::crc32(&bytes) == ssd_rom::CANONICAL_CRC32 => {
            Ok(RomSetup::Ready(bytes))
        }
        Ok(_) => {
            anyhow::bail!("cached ROM at {} is damaged: run again with --rom", cache.display())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RomSetup::Missing),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_rom_without_cache_is_reported_not_panicking() {
        let _guard =
            crate::paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        assert!(matches!(resolve_rom(None, None).unwrap(), RomSetup::Missing));
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn rejected_rom_reports_crc_and_writes_nothing() {
        let _guard =
            crate::paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let bogus = dir.path().join("bogus.sfc");
        std::fs::write(&bogus, vec![0x42_u8; 0x8000]).unwrap();
        let err = resolve_rom(Some(&bogus), None).unwrap_err();
        assert!(err.to_string().contains("unknown ROM"));
        assert!(!crate::paths::cached_rom().exists());
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn damaged_cache_is_rejected_with_a_hint() {
        let _guard =
            crate::paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let cache = crate::paths::cached_rom();
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, vec![0x42_u8; 0x8000]).unwrap();
        let err = resolve_rom(None, None).unwrap_err();
        assert!(err.to_string().contains("damaged"));
        assert!(err.to_string().contains("--rom"));
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }
}
