//! The language of a run's text (`ssd_menus::Language`): the PC file's choice (the Accessibility page's Language
//! row, or the first launch's picker), unless `--lang` names one for the run. Only the Enhanced mode draws a
//! translation; Classic is the original's English whatever the choice.

use ssd_menus::Language;
use ssd_menus::options::Values;
use std::sync::Mutex;

/// The language `--lang` gave, which wins over the PC file for the run.
static OVERRIDE: Mutex<Option<Language>> = Mutex::new(None);

/// Sets the language of the command line (`None`: the PC file's).
pub fn configure(language: Option<Language>) {
    *OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = language;
}

/// The language the command line gave, if it gave one.
pub fn overridden() -> Option<Language> {
    *OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The language of a run whose PC file holds `values`: the command line's, else the file's, else English.
pub fn effective(values: &Values) -> Language {
    overridden().or_else(|| Language::from_values(values)).unwrap_or_default()
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
}
