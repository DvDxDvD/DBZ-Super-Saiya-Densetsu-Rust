//! The language of a run's text (`ssd_menus::Language`): the PC file's choice (the Accessibility page's Language
//! row, or the first launch's picker), unless `--lang` names one for the run. Only the Enhanced mode draws a
//! translation; Classic is the original's English whatever the choice.
//!
//! The translation is the player's file, read at run time (nothing of it is built in): `zh-hk.tsv`
//! (`ssd_catalogue::translation`) in the data folder (`SSD_DATA_DIR`, the app's folder on Android), else next to the
//! executable. Without it, or for a key it lacks, the text stays English; a missing file is said once in the log.

use crate::paths;
use ssd_catalogue::translation::Translation;
use ssd_menus::Language;
use ssd_menus::options::Values;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The file of the Traditional Chinese (Hong Kong) translation.
pub const TABLE_FILE: &str = "zh-hk.tsv";

/// The language `--lang` gave, which wins over the PC file for the run.
static OVERRIDE: Mutex<Option<Language>> = Mutex::new(None);
/// The translation, read on first use (`None` inside: looked for and not found, or unreadable), and forgotten by
/// [`configure`]: Android runs the game more than once in a process.
static TABLE: Mutex<Lookup<Arc<Translation>>> = Mutex::new(Lookup::Unread);

/// A file looked for once: not yet, or found (`Some`) or not.
enum Lookup<T> {
    Unread,
    Read(Option<T>),
}

/// The value of `slot`, read with `read` on first use.
fn looked_up<T: Clone>(slot: &Mutex<Lookup<T>>, read: impl FnOnce() -> Option<T>) -> Option<T> {
    let mut slot = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Lookup::Unread = *slot {
        *slot = Lookup::Read(read());
    }
    match &*slot {
        Lookup::Read(value) => value.clone(),
        Lookup::Unread => None,
    }
}

/// Sets the language of the command line (`None`: the PC file's), and forgets the files read by an earlier run.
pub fn configure(language: Option<Language>) {
    *OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = language;
    *TABLE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Lookup::Unread;
}
/// The language the command line gave, if it gave one.
pub fn overridden() -> Option<Language> {
    *OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The language of a run whose PC file holds `values`: the command line's, else the file's, else English.
pub fn effective(values: &Values) -> Language {
    overridden().or_else(|| Language::from_values(values)).unwrap_or_default()
}

/// The folders the player's files of the translation are looked for in, in order: the data folder, then the
/// executable's.
fn places() -> Vec<PathBuf> {
    let mut places = vec![paths::data_dir()];
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(PathBuf::from))
    {
        places.push(dir);
    }
    places
}

/// The first of `places` that holds the file `name`.
fn find_in(places: &[PathBuf], name: &str) -> Option<PathBuf> {
    places.iter().map(|dir| dir.join(name)).find(|path| path.is_file())
}

/// The translation of `language`, read on first use; `None` for English, and when the file is missing or cannot be
/// read (said once in the log: the text stays English).
pub fn translation(language: Language) -> Option<Arc<Translation>> {
    if language != Language::ZhHk {
        return None;
    }
    looked_up(&TABLE, || {
        let places = places();
        let Some(path) = find_in(&places, TABLE_FILE) else {
            let looked: Vec<String> = places.iter().map(|dir| dir.display().to_string()).collect();
            log::warn!("no {TABLE_FILE} in {}: the text stays English", looked.join(" or "));
            return None;
        };
        match Translation::read(&path) {
            Ok(table) => {
                log::info!("translation: {} texts from {}", table.len(), path.display());
                Some(Arc::new(table))
            }
            Err(error) => {
                log::warn!("{}: {error}; the text stays English", path.display());
                None
            }
        }
    })
}

/// The tests that set the process-wide override (or depend on it) hold this lock.
#[cfg(test)]
pub(crate) static LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_wins_over_the_file_and_english_is_the_default() {
        let _guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        configure(None);
        let mut values = Values::new();
        assert_eq!(effective(&values), Language::English);
        Language::ZhHk.store(&mut values);
        assert_eq!(effective(&values), Language::ZhHk);
        configure(Some(Language::English));
        assert_eq!(effective(&values), Language::English);
        configure(None);
    }

    #[test]
    fn the_translation_is_read_from_the_first_folder_that_has_it_and_english_reads_none() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let places = [first.path().to_path_buf(), second.path().to_path_buf()];
        assert_eq!(find_in(&places, TABLE_FILE), None);
        std::fs::write(second.path().join(TABLE_FILE), "key\tzh\na\t甲\n").unwrap();
        assert_eq!(find_in(&places, TABLE_FILE), Some(second.path().join(TABLE_FILE)));
        std::fs::write(first.path().join(TABLE_FILE), "key\tzh\n").unwrap();
        assert_eq!(find_in(&places, TABLE_FILE), Some(first.path().join(TABLE_FILE)));
        assert_eq!(translation(Language::English), None);
    }
}
