//! The language of a run's text (`ssd_menus::Language`): the PC file's choice (the Accessibility page's Language
//! row, or the first launch's picker), unless `--lang` names one for the run. Only the Enhanced mode draws a
//! translation; Classic is the original's English whatever the choice.
//!
//! The translation is the player's file, read at run time (nothing of it is built in): `zh-hk.tsv`
//! (`ssd_catalogue::translation`) in the data folder (`SSD_DATA_DIR`, the app's folder on Android), else next to the
//! executable. Without it, or for a key it lacks, the text stays English; a missing file is said once in the log.

use crate::paths;
use ssd_catalogue::translation::Translation;
use ssd_hd::translate::{ChineseFace, Overlay};
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
/// The Chinese font's bytes, read on first use, kept as [`TABLE`] is (but not forgotten: the windows' fonts are
/// made from it).
static FONT: Mutex<Lookup<Arc<[u8]>>> = Mutex::new(Lookup::Unread);
/// The font made from [`FONT`]'s bytes for the translation's text, on first use.
static FACE: Mutex<Lookup<Arc<ChineseFace>>> = Mutex::new(Lookup::Unread);

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

/// The font files a Traditional Chinese face is taken from, in the `fonts` folder of a place: Noto Sans HK (SIL OFL),
/// else Noto Sans TC, the regular weight first. Fonts of the player's, never built in.
fn font_rank(name: &str) -> Option<u8> {
    let extension = std::path::Path::new(name).extension()?;
    if !(extension.eq_ignore_ascii_case("ttf") || extension.eq_ignore_ascii_case("otf")) {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    let family = if lower.starts_with("notosanshk") {
        0
    } else if lower.starts_with("notosanstc") {
        2
    } else {
        return None;
    };
    Some(family + u8::from(!lower.contains("-regular")))
}

/// The best Chinese font file of the `fonts` folders of `places` (see [`font_rank`]).
fn find_font(places: &[PathBuf]) -> Option<PathBuf> {
    places
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir.join("fonts")).ok())
        .flat_map(|entries| entries.flatten().map(|entry| entry.path()))
        .filter_map(|path| {
            let rank = font_rank(path.file_name()?.to_str()?)?;
            Some((rank, path))
        })
        .min_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, path)| path)
}

/// The bytes of the Traditional Chinese font (see [`font_rank`]), read on first use; `None` when there is none (said
/// once in the log). The HD menus take it as a fallback of their font, and the translation is drawn with it.
pub fn cjk_font() -> Option<Arc<[u8]>> {
    looked_up(&FONT, || {
        let Some(path) = find_font(&places()) else {
            log::info!(
                "no fonts/NotoSansHK-*.otf in the data folder: no Chinese text can be drawn"
            );
            return None;
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                log::info!("Chinese font: {}", path.display());
                Some(Arc::from(bytes))
            }
            Err(error) => {
                log::warn!("{}: {error}", path.display());
                None
            }
        }
    })
}

/// The overlay that draws the translation of `language` over the game's message windows (`ssd_hd::translate`):
/// `None` for English, and without the translation or the font (the text stays English).
pub fn overlay(language: Language) -> Option<Arc<Overlay>> {
    let table = translation(language)?;
    let face = looked_up(&FACE, || {
        let bytes = cjk_font()?;
        match ChineseFace::new(bytes.to_vec()) {
            Ok(face) => Some(Arc::new(face)),
            Err(error) => {
                log::warn!("the Chinese font cannot be read ({error}): the text stays English");
                None
            }
        }
    })?;
    Some(Arc::new(Overlay::new(table, face)))
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

    #[test]
    fn the_font_is_noto_sans_hk_regular_first_then_another_weight_then_tc() {
        assert_eq!(font_rank("NotoSansHK-Regular.otf"), Some(0));
        assert_eq!(font_rank("NotoSansHK-Bold.ttf"), Some(1));
        assert_eq!(font_rank("NotoSansTC-Regular.ttf"), Some(2));
        assert_eq!(font_rank("NotoSansHK-Regular.woff2"), None);
        assert_eq!(font_rank("OFL.txt"), None);
        let dir = tempfile::tempdir().unwrap();
        let fonts = dir.path().join("fonts");
        std::fs::create_dir(&fonts).unwrap();
        let places = [dir.path().to_path_buf()];
        assert_eq!(find_font(&places), None);
        for name in ["NotoSansTC-Regular.otf", "NotoSansHK-Medium.otf", "OFL.txt"] {
            std::fs::write(fonts.join(name), b"x").unwrap();
        }
        assert_eq!(find_font(&places), Some(fonts.join("NotoSansHK-Medium.otf")));
    }
}
