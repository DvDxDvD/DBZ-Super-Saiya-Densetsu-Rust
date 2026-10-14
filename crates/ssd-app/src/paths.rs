use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SSD_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
    base.join("dbz-ssd-recompiled")
}

pub fn cached_rom() -> PathBuf {
    data_dir().join("cache").join("rom").join("canonical.sfc")
}

pub fn saves_dir() -> PathBuf {
    data_dir().join("saves")
}

pub fn states_dir() -> PathBuf {
    data_dir().join("states")
}

/// Every test that touches the process-wide `SSD_DATA_DIR` holds this lock.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    #[test]
    fn data_dir_honours_override() {
        let _guard = super::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        assert_eq!(super::data_dir(), dir.path());
        assert_eq!(super::cached_rom(), dir.path().join("cache").join("rom").join("canonical.sfc"));
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }
}
