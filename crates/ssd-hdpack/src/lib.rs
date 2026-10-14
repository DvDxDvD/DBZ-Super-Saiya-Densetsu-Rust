//! The HD pack of a ROM: `convert` builds the base pack from the ROM alone, `import` puts an art
//! store (or a shared HD art pack, `public`) into it, and `install` does both for a player on the
//! first launch. Nothing the ROM holds is written anywhere but under the data directory's cache.

pub mod convert;
pub mod import;
pub mod install;
pub mod public;
pub mod tiles;

/// The canonical ROM of the data directory's cache, for the tests that need the game.
#[cfg(test)]
pub(crate) fn cached_rom() -> anyhow::Result<Vec<u8>> {
    let dir = std::env::var_os("SSD_DATA_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
            .join("dbz-ssd-recompiled")
    });
    Ok(std::fs::read(dir.join("cache").join("rom").join("canonical.sfc"))?)
}
