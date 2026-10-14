//! The HD art of a run: the pack the converter and `hd-import` left for the ROM, found once and
//! shared by the Enhanced picture and the HD menus. `--hd-pack` names another pack folder and
//! `--no-hd-art` turns the art off; without art the Enhanced picture is the look alone. The HD text
//! font (the game's UI text redrawn from Share Tech Mono at the picture's scale) is found here
//! too; `--no-hd-font` turns it off.

use ssd_hd::Font;
use ssd_hd::pack::Pack;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// What the command line said about the art.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Use this pack folder instead of the newest one under the cache directory.
    pub pack: Option<PathBuf>,
    /// No art at all.
    pub off: bool,
    /// No HD text: the game's own letters, made bigger by the look.
    pub font_off: bool,
}

static CONFIG: Mutex<Config> = Mutex::new(Config { pack: None, off: false, font_off: false });
static PACK: OnceLock<Option<Arc<Pack>>> = OnceLock::new();
static FONT: OnceLock<Option<Arc<Font>>> = OnceLock::new();

/// Sets what the command line said (before the first [`pack`]).
pub fn configure(config: Config) {
    *CONFIG.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = config;
}

/// The pack with the HD art for `rom`, found on first use; `None` when art is off, there is no
/// pack for the ROM, or the pack holds no art.
pub fn pack(rom: &[u8]) -> Option<Arc<Pack>> {
    PACK.get_or_init(|| {
        let config = CONFIG.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        if config.off {
            return None;
        }
        let found = match &config.pack {
            Some(dir) => match Pack::open(dir) {
                Ok(pack) => Some(pack),
                Err(error) => {
                    log::warn!("the HD pack {}: {error}", dir.display());
                    None
                }
            },
            None => Pack::find(&crate::paths::data_dir().join("cache"), ssd_rom::crc32(rom)),
        };
        let pack = found
            .filter(|pack| !pack.art.pictures().is_empty() || !pack.art.objects().is_empty())?;
        log::info!(
            "HD art: {} pictures, {} poses from {}",
            pack.art.pictures().len(),
            pack.art.objects().len(),
            pack.dir.display()
        );
        Some(Arc::new(pack))
    })
    .clone()
}

/// The HD text font for `rom`, made on first use; `None` when it is off or the image has no UI font.
pub fn font(rom: &[u8]) -> Option<Arc<Font>> {
    FONT.get_or_init(|| {
        if CONFIG.lock().unwrap_or_else(std::sync::PoisonError::into_inner).font_off {
            return None;
        }
        // The card art lives in the pack: with no art (or none for this ROM) the cards stay as drawn
        let pack = pack(rom);
        match Font::standard_with_pack(rom, pack.as_ref().map(|pack| pack.dir.as_path())) {
            Ok(font) => Some(Arc::new(font)),
            Err(error) => {
                log::warn!("the HD text font: {error}");
                None
            }
        }
    })
    .clone()
}
