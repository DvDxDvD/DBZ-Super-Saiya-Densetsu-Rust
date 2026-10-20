//! PNG screenshots: where they go, how they are named, and how they are written. A screenshot is
//! not a save: a windowed run may take one at any time, and a run that does not persist (headless,
//! UI snapshot, replay check) takes none.
//!
//! The folder is `<Pictures>/DBZ SSD Recompiled/` (the system's Pictures folder, as SDL names it);
//! when the system has none, and on Android, where the Pictures folder is not the app's to write,
//! it is `screenshots/` of the data folder. `SSD_SCREENSHOT_DIR` replaces the folder (the tests).

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The folder made under the Pictures folder.
pub const FOLDER: &str = "DBZ SSD Recompiled";
/// The variable that replaces the folder of the screenshots.
pub const DIR_ENV: &str = "SSD_SCREENSHOT_DIR";

/// Which picture a screenshot takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The picture of the window as the player sees it, without the debug panel.
    Window,
    /// The PPU's own frame, unscaled (256 pixels wide).
    Frame,
}

/// A calendar time, local, to the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Stamp {
    /// The local time now (UTC when the system cannot tell).
    #[must_use]
    pub fn now() -> Self {
        local_now().unwrap_or_else(|| utc_of(unix_seconds()))
    }
}

/// The name of the `n`th screenshot taken in the second of `stamp` (`n` from 1): `dbz-YYYYMMDD-HHMMSS.png`,
/// then `dbz-YYYYMMDD-HHMMSS-2.png` and so on.
#[must_use]
pub fn file_name(stamp: Stamp, n: u32) -> String {
    let Stamp { year, month, day, hour, minute, second } = stamp;
    let base = format!("dbz-{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}");
    if n <= 1 { format!("{base}.png") } else { format!("{base}-{n}.png") }
}

/// The folder the screenshots go to.
#[must_use]
pub fn folder() -> PathBuf {
    if let Some(dir) = std::env::var_os(DIR_ENV) {
        return PathBuf::from(dir);
    }
    pictures().map_or_else(|| crate::paths::data_dir().join("screenshots"), |dir| dir.join(FOLDER))
}

#[cfg(not(target_os = "android"))]
fn pictures() -> Option<PathBuf> {
    sdl3::filesystem::get_user_folder(sdl3::filesystem::Folder::PICTURES)
        .ok()
        .map(Path::to_path_buf)
}

#[cfg(target_os = "android")]
fn pictures() -> Option<PathBuf> {
    None
}

/// Writes `rgba` (`width` x `height`, top row first) as a new PNG in `dir`, named for `stamp`; a file
/// that exists is never replaced, the next number is tried. Returns the path written.
///
/// # Errors
///
/// The folder or the file cannot be written.
pub fn save(
    dir: &Path,
    stamp: Stamp,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let bytes = ssd_hd::png::encode_rgba_compressed(width, height, rgba);
    for n in 1..10_000 {
        let path = dir.join(file_name(stamp, n));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("too many screenshots in the same second"))
}

/// The name of the state taken beside the screenshot at `png`: the same name with `.ssdstate`.
#[must_use]
pub fn state_name(png: &Path) -> PathBuf {
    png.with_extension("ssdstate")
}

/// Writes `state` (the machine's own bytes, as `states\slotN.ssdstate`) beside the screenshot at
/// `png`, through a temporary file so a crash leaves no truncated state. Returns the path written.
///
/// # Errors
///
/// The folder or the file cannot be written.
pub fn save_state(png: &Path, state: &[u8]) -> std::io::Result<PathBuf> {
    let path = state_name(png);
    let tmp = path.with_extension("ssdstate.tmp");
    let written = std::fs::write(&tmp, state)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| std::io::Error::new(e.kind(), format!("writing {}: {e}", path.display())));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.map(|()| path)
}

/// The pixels of a frame as RGBA bytes.
#[must_use]
pub fn frame_rgba(frame: &ssd_oracle::Frame<'_>) -> Vec<u8> {
    frame.pixels.iter().flat_map(|c| [c.r, c.g, c.b, 255]).collect()
}

/// The text of the notice after a screenshot (the picture and the state beside it).
#[must_use]
pub fn notice(result: &std::io::Result<(PathBuf, PathBuf)>) -> String {
    match result {
        Ok((png, _state)) => format!("Screenshot + state saved: {}", png.display()),
        Err(e) => format!("Screenshot failed: {e}"),
    }
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// The UTC calendar time of `seconds` since the Unix epoch.
#[must_use]
pub fn utc_of(seconds: i64) -> Stamp {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    Stamp {
        year: i32::try_from(year).unwrap_or(1970),
        month: u32::try_from(month).unwrap_or(1),
        day: u32::try_from(day).unwrap_or(1),
        hour: u32::try_from(rest / 3600).unwrap_or(0),
        minute: u32::try_from(rest % 3600 / 60).unwrap_or(0),
        second: u32::try_from(rest % 60).unwrap_or(0),
    }
}

/// The local time by SDL.
#[allow(unsafe_code)]
fn local_now() -> Option<Stamp> {
    use sdl3::sys::time::{SDL_DateTime, SDL_GetCurrentTime, SDL_TimeToDateTime};
    let mut ticks = 0;
    let mut dt = SDL_DateTime {
        year: 0,
        month: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        nanosecond: 0,
        day_of_week: 0,
        utc_offset: 0,
    };
    // SAFETY: both calls write only through the pointers given, which are valid for the call.
    let ok = unsafe {
        SDL_GetCurrentTime(&raw mut ticks) && SDL_TimeToDateTime(ticks, &raw mut dt, true)
    };
    ok.then(|| Stamp {
        year: dt.year,
        month: u32::try_from(dt.month).unwrap_or(1),
        day: u32::try_from(dt.day).unwrap_or(1),
        hour: u32::try_from(dt.hour).unwrap_or(0),
        minute: u32::try_from(dt.minute).unwrap_or(0),
        second: u32::try_from(dt.second).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAMP: Stamp = Stamp { year: 2026, month: 10, day: 16, hour: 9, minute: 5, second: 3 };

    #[test]
    fn a_file_is_named_for_its_second_and_numbered_from_the_second_one() {
        assert_eq!(file_name(STAMP, 1), "dbz-20261016-090503.png");
        assert_eq!(file_name(STAMP, 2), "dbz-20261016-090503-2.png");
        assert_eq!(file_name(STAMP, 10), "dbz-20261016-090503-10.png");
    }

    #[test]
    fn utc_dates_come_from_the_epoch() {
        assert_eq!(
            utc_of(0),
            Stamp { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0 }
        );
        assert_eq!(
            utc_of(1_700_000_000),
            Stamp { year: 2023, month: 11, day: 14, hour: 22, minute: 13, second: 20 }
        );
        assert_eq!(
            utc_of(951_782_400),
            Stamp { year: 2000, month: 2, day: 29, hour: 0, minute: 0, second: 0 }
        );
    }

    #[test]
    fn a_screenshot_never_replaces_another() {
        let dir = tempfile::tempdir().unwrap();
        let rgba = [10_u8, 20, 30, 255, 40, 50, 60, 255];
        let first = save(&dir.path().join("new").join("shots"), STAMP, 2, 1, &rgba).unwrap();
        let folder = first.parent().unwrap().to_path_buf();
        let second = save(&folder, STAMP, 2, 1, &rgba).unwrap();
        let third = save(&folder, STAMP, 2, 1, &rgba).unwrap();
        assert_eq!(first.file_name().unwrap(), "dbz-20261016-090503.png");
        assert_eq!(second.file_name().unwrap(), "dbz-20261016-090503-2.png");
        assert_eq!(third.file_name().unwrap(), "dbz-20261016-090503-3.png");
        let (w, h, pixels) = ssd_hd::png::read_rgba(&std::fs::read(&first).unwrap()).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(pixels, rgba);
    }

    #[test]
    fn the_ppu_frame_is_saved_at_its_own_size() {
        let (machine, _store) = crate::test_support::sram_counter_machine();
        let frame = machine.frame();
        let (w, h) = (frame.size.width, frame.size.height);
        let rgba = frame_rgba(&frame);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        let dir = tempfile::tempdir().unwrap();
        let path = save(dir.path(), STAMP, w, h, &rgba).unwrap();
        let (sw, sh, pixels) = ssd_hd::png::read_rgba(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!((sw, sh), (w, h));
        assert_eq!(pixels, rgba);
    }
    #[test]
    fn the_folder_can_be_replaced_and_is_under_pictures_otherwise() {
        let _guard =
            crate::paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var(DIR_ENV, dir.path()) };
        assert_eq!(folder(), dir.path());
        unsafe { std::env::remove_var(DIR_ENV) };
        let folder = folder();
        assert!(
            folder.ends_with(FOLDER) || folder.ends_with("screenshots"),
            "{}",
            folder.display()
        );
    }

    #[test]
    fn the_notice_names_the_path_or_the_error() {
        let ok = Ok((PathBuf::from("a/b.png"), PathBuf::from("a/b.ssdstate")));
        assert!(notice(&ok).starts_with("Screenshot + state saved: "));
        let failed = Err(std::io::Error::other("disk full"));
        assert_eq!(notice(&failed), "Screenshot failed: disk full");
    }

    #[test]
    fn the_state_is_written_beside_the_screenshot() {
        let dir = tempfile::tempdir().unwrap();
        let rgba = [10_u8, 20, 30, 255, 40, 50, 60, 255];
        let png = save(dir.path(), STAMP, 2, 1, &rgba).unwrap();
        let state = [1_u8, 2, 3, 4, 5];
        let path = save_state(&png, &state).unwrap();
        assert_eq!(path.file_name().unwrap(), "dbz-20261016-090503.ssdstate");
        assert_eq!(std::fs::read(&path).unwrap(), state);
    }

    #[test]
    fn a_screenshot_is_written_compressed() {
        // A flat image stores to almost nothing under stored deflate too, so use noise.
        let (w, h) = (64_u32, 64_u32);
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| u8::try_from(i % 251).unwrap()).collect();
        let stored = ssd_hd::png::encode_rgba(w, h, &rgba);
        let compressed = ssd_hd::png::encode_rgba_compressed(w, h, &rgba);
        assert!(compressed.len() < stored.len(), "{} !< {}", compressed.len(), stored.len());
        let (dw, dh, pixels) = ssd_hd::png::read_rgba(&compressed).unwrap();
        assert_eq!((dw, dh), (w, h));
        assert_eq!(pixels, rgba);
    }
}
