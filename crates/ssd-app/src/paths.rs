use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether this run writes the player's files: the battery save, the bag and cheat files, the save states and the
/// option files. Off until `main` turns it on for a windowed run (see [`set_persistent`]), so nothing a test
/// writes reaches the saves folder unless the test asks for it.
static PERSISTENT: AtomicBool = AtomicBool::new(false);

/// Sets whether this run writes the player's files. `main` sets it once, before anything is read.
pub fn set_persistent(persistent: bool) {
    PERSISTENT.store(persistent, Ordering::Relaxed);
}

/// Whether this run writes the player's files. A run that does not persist (a headless run, a UI snapshot, a
/// benchmark, a replay) reads the files that exist but writes none, and keeps its changes in memory (README,
/// "Data and files").
pub fn persists() -> bool {
    PERSISTENT.load(Ordering::Relaxed)
}

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
