//! The files of the options (M7): `ssd.options` (Difficulty and Gameplay, one entry per save slot) and
//! `ssd.settings` (Video, Audio, Controls and Accessibility, for this PC), both in the saves directory, and the
//! saved video choices a windowed run starts with (`video`). The model and the text formats are `ssd_menus::options`.
//! Only a windowed run reads and writes the two files (see [`paths::persists`]).

use crate::paths;
use anyhow::Context;
use ssd_menus::options::{SlotOptions, Values};
use std::path::{Path, PathBuf};

pub mod access;

pub mod audio;

pub mod controls;

pub mod video;

const SLOTS_FILE: &str = "ssd.options";
const SETTINGS_FILE: &str = "ssd.settings";

/// The slot file's path.
fn slots_path() -> PathBuf {
    paths::saves_dir().join(SLOTS_FILE)
}

/// The PC file's path.
fn settings_path() -> PathBuf {
    paths::saves_dir().join(SETTINGS_FILE)
}

/// The text of `path`: `None` when the file does not exist, and when it cannot be read as `parse` wants it, the
/// file is kept aside as `.bad` and the defaults apply (the run is not refused).
fn read_or_aside<T>(
    path: &Path,
    parse: impl Fn(&str) -> Result<T, String>,
) -> anyhow::Result<Option<T>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(match parse(&text) {
            Ok(value) => Some(value),
            Err(error) => {
                let mut aside = path.as_os_str().to_owned();
                aside.push(".bad");
                log::warn!("{}: {error}; kept as {}", path.display(), Path::new(&aside).display());
                if let Err(error) = std::fs::rename(path, &aside) {
                    log::warn!("{error}");
                }
                None
            }
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// The options of the save slots: none when the file does not exist yet, and none in a run that does not persist.
///
/// # Errors
///
/// The file exists and cannot be read.
pub fn read_slots() -> anyhow::Result<SlotOptions> {
    if !paths::persists() {
        return Ok(SlotOptions::empty());
    }
    Ok(read_or_aside(&slots_path(), SlotOptions::from_text)?.unwrap_or_default())
}

/// The options kept for this PC: none when the file does not exist yet, and none in a run that does not persist.
///
/// # Errors
///
/// The file exists and cannot be read.
pub fn read_settings() -> anyhow::Result<Values> {
    if !paths::persists() {
        return Ok(Values::new());
    }
    Ok(read_or_aside(&settings_path(), Values::from_text)?.unwrap_or_default())
}

/// Writes the slot file; a run that does not persist writes nothing.
///
/// # Errors
///
/// The file cannot be written.
pub fn write_slots(slots: &SlotOptions) -> anyhow::Result<()> {
    crate::hd::write_beside(&slots_path(), slots.to_text().as_bytes())
}

/// Writes the PC file; a run that does not persist writes nothing.
///
/// # Errors
///
/// The file cannot be written.
pub fn write_settings(values: &Values) -> anyhow::Result<()> {
    crate::hd::write_beside(&settings_path(), values.to_text().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_round_trip_and_a_damaged_one_is_kept_aside() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        paths::set_persistent(true);
        assert_eq!(read_settings().unwrap(), Values::new(), "no file: the defaults");
        let mut values = Values::new();
        values.set("video.scale", 3);
        write_settings(&values).unwrap();
        assert_eq!(read_settings().unwrap(), values);
        let mut slots = SlotOptions::empty();
        slots.store(2, 99, values.clone());
        write_slots(&slots).unwrap();
        assert_eq!(read_slots().unwrap(), slots);
        std::fs::write(slots_path(), "garbage").unwrap();
        assert_eq!(read_slots().unwrap(), SlotOptions::empty());
        assert!(slots_path().with_file_name("ssd.options.bad").exists());
        assert!(!slots_path().exists(), "the damaged file was moved");
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn a_run_that_does_not_persist_reads_and_writes_no_option_file() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        // A windowed run wrote both files.
        paths::set_persistent(true);
        let mut values = Values::new();
        values.set("video.scale", 3);
        write_settings(&values).unwrap();
        let mut slots = SlotOptions::empty();
        slots.store(2, 99, values.clone());
        write_slots(&slots).unwrap();
        let (settings_text, slots_text) =
            (std::fs::read(settings_path()).unwrap(), std::fs::read(slots_path()).unwrap());
        // A headless run, a UI snapshot, a benchmark or a replay: it reads neither file, and its
        // changes stay in memory.
        paths::set_persistent(false);
        assert_eq!(read_settings().unwrap(), Values::new(), "the PC file is not read");
        assert_eq!(read_slots().unwrap(), SlotOptions::empty(), "the slot file is not read");
        let mut changed = Values::new();
        changed.set("video.scale", 5);
        write_settings(&changed).unwrap();
        slots.store(2, 99, changed.clone());
        write_slots(&slots).unwrap();
        assert_eq!(
            std::fs::read(settings_path()).unwrap(),
            settings_text,
            "the PC file is unchanged"
        );
        assert_eq!(std::fs::read(slots_path()).unwrap(), slots_text, "the slot file is unchanged");
        // Files that do not exist yet do not appear.
        std::fs::remove_file(settings_path()).unwrap();
        std::fs::remove_file(slots_path()).unwrap();
        write_settings(&changed).unwrap();
        write_slots(&slots).unwrap();
        assert!(!settings_path().exists() && !slots_path().exists(), "no option file appears");
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }
}
