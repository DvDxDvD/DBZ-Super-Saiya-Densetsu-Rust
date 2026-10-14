//! The first-launch install of the shared HD art pack: when `hd-pack.zip` (or an `hd-pack` folder)
//! sits next to the game, or `--install-hd-pack` names one, the HD pack of the player's ROM is built
//! from the ROM and that art before the game starts (`ssd_hdpack::install`), once. Classic needs
//! nothing and never gets here.

use ssd_hdpack::install::{self, Outcome};
use std::path::Path;

/// What the launch asked of the install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Not asked: Classic, or art turned off.
    No,
    /// The HD picture or the HD menus will draw art: install the pack found next to the game.
    Wanted,
    /// `--install-hd-pack`: the player named a pack, and a failure is an error.
    Asked,
}

/// Why the install stopped the launch.
#[derive(Debug)]
pub struct Failed(pub String);

/// Installs the shared art pack if one is there and the pack of the ROM is not made from it yet.
///
/// # Errors
///
/// The player named a pack (`explicit`) that cannot be installed, or none was found for it. A pack
/// that was only found next to the game and fails to install is a warning: the game starts without
/// the art.
pub fn ensure(rom: &[u8], need: Need, explicit: Option<&Path>) -> Result<(), Failed> {
    if need == Need::No {
        return Ok(());
    }
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    let Some(source) = install::find_source(explicit, exe_dir.as_deref()) else {
        return match explicit {
            Some(path) => {
                Err(Failed(format!("no HD pack found at {} (a zip or a folder)", path.display())))
            }
            None => Ok(()),
        };
    };
    let cache = crate::paths::data_dir().join("cache");
    let mut say = |line: &str| eprintln!("{line}");
    match install::install(rom, &source, &cache, &mut say) {
        Ok(Outcome::Current(pack)) => {
            log::info!("HD pack: {} is made from {}", pack.display(), source.display());
            Ok(())
        }
        Ok(Outcome::Installed { pack, report }) => {
            log::info!(
                "HD pack: {} portraits, {} poses, {} map layers, {} backgrounds installed in {}",
                report.pictures,
                report.objects,
                report.map_layers,
                report.bg,
                pack.display()
            );
            Ok(())
        }
        Err(error) if need == Need::Asked => Err(Failed(format!("{error:#}"))),
        Err(error) => {
            log::warn!("HD pack {}: {error:#}", source.display());
            eprintln!("HD pack: not installed ({error:#}); the game starts without the HD art.");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_installed_when_nothing_is_asked_or_found() {
        assert!(ensure(&[], Need::No, None).is_ok());
        // Wanted with no pack beside the test executable: the game just starts
        assert!(ensure(&[], Need::Wanted, None).is_ok());
    }

    #[test]
    fn a_named_pack_that_is_not_there_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let Failed(message) =
            ensure(&[], Need::Asked, Some(&dir.path().join("nope.zip"))).unwrap_err();
        assert!(message.contains("nope.zip"), "{message}");
    }
}
