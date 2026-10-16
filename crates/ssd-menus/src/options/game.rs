//! The Game page (Android only; see [`super::show_game_page`]): the mode and the HD pack, which the setup screens
//! change. A button restarts the game into the setup page; the two lines say what is now.

use super::{Kind, Opens, RowDef, Source};

/// The rows of the page.
pub const ROWS: &[RowDef] = &[
    RowDef {
        key: "game.mode",
        label: "Current mode",
        group: "Mode",
        help: "The mode the game starts in.",
        kind: Kind::Info,
        source: Source::Info,
    },
    RowDef {
        key: "game.change_mode",
        label: "Change mode",
        group: "Mode",
        help: "Choose Classic or Enhanced. The game restarts into the choice; your saves stay.",
        kind: Kind::Button("Change..."),
        source: Source::Opens(Opens::Mode),
    },
    RowDef {
        key: "game.pack",
        label: "HD pack",
        group: "HD pack",
        help: "The art pack the Enhanced mode draws from.",
        kind: Kind::Info,
        source: Source::Info,
    },
    RowDef {
        key: "game.install_pack",
        label: "Install the HD pack",
        group: "HD pack",
        help: "Pick hd-pack.zip to install or update the art (about 0.9 GB). The game restarts into the install.",
        kind: Kind::Button("Install..."),
        source: Source::Opens(Opens::Pack),
    },
];

/// A line under the rows.
pub const NOTE: Option<&str> = Some("The game restarts when a button here opens the setup.");

/// The text of the read-only row `key`.
#[must_use]
pub fn info_text(key: &str) -> String {
    let info = super::GAME_INFO.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match key {
        "game.mode" => info.0.clone(),
        "game.pack" => info.1.clone(),
        _ => String::new(),
    }
}
