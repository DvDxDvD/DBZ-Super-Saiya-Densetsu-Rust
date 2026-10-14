//! The accounting of `hd-tour`: which class of asset each visible pixel of a frame belongs to, how
//! many of them the HD art draws, and what is left to draw (the items with no art, with where
//! they were first seen). Pure arithmetic over the tallies `ssd-hd` makes ([`FrameTally`]), so it
//! is unit-tested without a ROM.
//!
//! A pixel's class follows from what the tally says about it (its layer, the pack asset its
//! character is bound to, the pose it belongs to) and from the scene the game is in (battle or
//! not); the rules are in [`classify_pixel`]. The art draws a pixel when the art index has art for
//! the picture or the pose it belongs to (palette variants count when the art index finds them).

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use ssd_hd::pack::Index;
use ssd_hd::{FrameTally, ItemKey, TallyCount, TallyLayer};
use std::collections::BTreeMap;

/// The classes of assets a tour reports, as the art plan names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Portraits and faces (the pack's `portrait` assets).
    Portraits,
    /// The fighters' poses in a battle.
    Fighters,
    /// The effects of a battle animation (ki blasts, impacts, big attack sprites).
    Effects,
    /// Sprites of the field: the party, the people and objects of a map, objects of other scenes.
    FieldSprites,
    /// The tiles of the field map and the Mode 7 world of the flights.
    Map,
    /// The backgrounds of a battle.
    BattleBg,
    /// Big pictures, cutscenes, the title and the endings.
    Pictures,
    /// The glyphs of the UI tile set: the font of every text window.
    Text,
    /// Cards, window frames, cursors and the rest of the UI tile set and the battle's menus.
    CardsUi,
}

/// Every class, in the order of the report.
pub const CLASSES: [Class; 9] = [
    Class::Portraits,
    Class::Fighters,
    Class::Effects,
    Class::FieldSprites,
    Class::Map,
    Class::BattleBg,
    Class::Pictures,
    Class::Text,
    Class::CardsUi,
];

impl Class {
    /// The name of the class in options, the config and `coverage.json`.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Portraits => "portraits",
            Self::Fighters => "fighters",
            Self::Effects => "effects",
            Self::FieldSprites => "field_sprites",
            Self::Map => "map",
            Self::BattleBg => "battle_bg",
            Self::Pictures => "pictures",
            Self::Text => "text",
            Self::CardsUi => "cards_ui",
        }
    }

    /// What the report calls the class.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Portraits => "portraits/faces",
            Self::Fighters => "battle fighters",
            Self::Effects => "battle effects",
            Self::FieldSprites => "field sprites",
            Self::Map => "map tiles",
            Self::BattleBg => "battle backgrounds",
            Self::Pictures => "big pictures",
            Self::Text => "fonts/text",
            Self::CardsUi => "cards/UI",
        }
    }

    /// The class named `name` (its [`Class::key`]).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        CLASSES.iter().copied().find(|class| class.key() == name)
    }
}

/// The tiles of the UI tile set (`ui/11:8036`) that are the font's glyphs: the letters, kana and
/// digits are tiles 4 to 143 but for the pieces of window frames among them
/// ([`UI_CHROME`]); tiles 0 to 3 are solid fills and the tiles from 144 on the cards, frames,
/// cursors and numerals. An approximation by eye of the tile set's sheet.
pub const UI_GLYPHS: std::ops::Range<u32> = 4..144;

/// The tiles inside [`UI_GLYPHS`] that are pieces of window frames and fills, not glyphs.
pub const UI_CHROME: [std::ops::RangeInclusive<u32>; 2] = [50..=54, 59..=61];

/// Whether tile `tile` of the UI tile set is a glyph of the font.
#[must_use]
pub fn is_glyph(tile: u32) -> bool {
    UI_GLYPHS.contains(&tile) && !UI_CHROME.iter().any(|chrome| chrome.contains(&tile))
}

/// What kind of stream an asset of the pack is, from its id (`<class>/<bank>:<offset>/<bits>bpp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ui,
    Field,
    Sprite,
    Portrait,
    Picture,
    Title,
    Fighter,
    Other,
}

/// What a tour knows of an asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo {
    pub id: String,
    pub kind: Kind,
    /// The ROM bank the stream comes from.
    pub bank: u8,
    /// Tiles per row of the asset's art image.
    pub columns: u16,
}

/// The assets of a pack, as far as the accounting needs them.
#[derive(Debug, Clone, Default)]
pub struct AssetTable {
    assets: Vec<AssetInfo>,
}

impl AssetTable {
    #[must_use]
    pub fn new(index: &Index) -> Self {
        Self { assets: index.assets().iter().map(|a| info_of(&a.id, a.columns)).collect() }
    }

    /// The table of the asset ids given (for tests).
    #[cfg(test)]
    #[must_use]
    pub fn of_ids(ids: &[&str]) -> Self {
        Self { assets: ids.iter().map(|id| info_of(id, 16)).collect() }
    }

    /// What kind of stream asset `asset` is.
    #[must_use]
    pub fn kind(&self, asset: u32) -> Option<Kind> {
        self.get(asset).map(|a| a.kind)
    }

    /// The logical id of asset `asset`.
    #[must_use]
    pub fn id(&self, asset: u32) -> &str {
        self.assets.get(asset as usize).map_or("?", |a| a.id.as_str())
    }

    fn get(&self, asset: u32) -> Option<&AssetInfo> {
        self.assets.get(asset as usize)
    }
}

fn info_of(id: &str, columns: u16) -> AssetInfo {
    let mut parts = id.split('/');
    let kind = match parts.next() {
        Some("ui") => Kind::Ui,
        Some("field") => Kind::Field,
        Some("sprite") => Kind::Sprite,
        Some("portrait") => Kind::Portrait,
        Some("picture") => Kind::Picture,
        Some("title") => Kind::Title,
        Some("fighter") => Kind::Fighter,
        _ => Kind::Other,
    };
    let bank = parts
        .next()
        .and_then(|source| source.split(':').next())
        .and_then(|bank| u8::from_str_radix(bank, 16).ok())
        .unwrap_or(0);
    AssetInfo { id: id.to_owned(), kind, bank, columns }
}

/// The scene a frame is in, as far as the classes care.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// A battle: the command screen, an attack animation, a last fight.
    pub battle: bool,
    /// An animation of the battle (an attack, a special attack, a last fight).
    pub animation: bool,
}

/// `game_mode` in work RAM.
const GAME_MODE: usize = 0x24;

impl Context {
    /// The context of the frame the work RAM `wram` ended: `game_mode` `$02` is the battle's
    /// command screen, `$04` an attack animation, `$0C` a last fight.
    #[must_use]
    pub fn of_wram(wram: &[u8]) -> Self {
        let mode = wram.get(GAME_MODE).map_or(0, |mode| mode & 0x7F);
        Self { battle: matches!(mode, 0x02 | 0x04 | 0x0C), animation: matches!(mode, 0x04 | 0x0C) }
    }
}

/// What the accounting needs to say of one pixel or item.
struct Pixel {
    layer: TallyLayer,
    bpp: u8,
    /// The asset it is bound to, with the tile of it.
    asset: Option<(Kind, u32)>,
    /// For an object pixel of a pose: whether it is a fighter's.
    pose: Option<bool>,
}

/// The class of a pixel (the rules of the tour).
///
/// * The portrait assets are the portraits, whatever the scene; the UI tile set is the text (its
///   glyph rows, [`UI_TEXT_ROWS`]) or the cards and UI (the rest).
/// * A Mode 7 plane is the battle's background in a battle and the map otherwise.
/// * A pose of the object layer is a fighter or an effect in a battle (the command screen's
///   other sprites are cursor and cards) and a field sprite elsewhere.
/// * A BG pixel is a battle background in a battle (the command screen's backdrop, the stage of an
///   animation, the flat colours and speed lines of a special attack), a map tile for the field's
///   tiles, a big picture for the pictures, the title and the fighters' pieces drawn as BG
///   elsewhere.
/// * An object that is not a pose is a field sprite, or in a battle an effect (animation) or a
///   card or cursor (command screen).
/// * A character bound to no asset of the pack is classed by its layer and the scene: a
///   2-bit BG character outside a battle is text, a 4-bit one the map.
fn classify_pixel(pixel: &Pixel, ctx: Context) -> Class {
    match (pixel.layer, pixel.pose) {
        (TallyLayer::Plane, _) => {
            return if ctx.battle { Class::BattleBg } else { Class::Map };
        }
        (TallyLayer::Obj, Some(true)) => {
            return if ctx.battle { Class::Fighters } else { Class::FieldSprites };
        }
        (TallyLayer::Obj, Some(false)) => {
            return match (ctx.battle, ctx.animation) {
                (false, _) => Class::FieldSprites,
                (true, true) => Class::Effects,
                (true, false) => Class::CardsUi,
            };
        }
        _ => {}
    }
    let object = pixel.layer == TallyLayer::Obj;
    if let Some((kind, tile)) = pixel.asset {
        match kind {
            Kind::Portrait => return Class::Portraits,
            Kind::Ui => return if is_glyph(tile) { Class::Text } else { Class::CardsUi },
            _ => {}
        }
        return match (object, ctx.battle) {
            (false, true) => Class::BattleBg,
            (false, false) => match kind {
                Kind::Field | Kind::Other => Class::Map,
                Kind::Sprite => Class::FieldSprites,
                _ => Class::Pictures,
            },
            (true, true) => {
                if ctx.animation {
                    Class::Effects
                } else {
                    Class::CardsUi
                }
            }
            (true, false) => match kind {
                Kind::Picture | Kind::Title => Class::Pictures,
                _ => Class::FieldSprites,
            },
        };
    }
    match (object, ctx.battle) {
        (true, true) => {
            if ctx.animation {
                Class::Effects
            } else {
                Class::CardsUi
            }
        }
        (true, false) => Class::FieldSprites,
        (false, true) => Class::BattleBg,
        (false, false) => {
            if pixel.bpp == 2 {
                Class::Text
            } else {
                Class::Map
            }
        }
    }
}

fn bound(table: &AssetTable, asset: Option<u32>, tile: u32) -> Option<(Kind, u32)> {
    let info = table.get(asset?)?;
    Some((info.kind, tile))
}

/// The class of the pixels a tally count describes.
#[must_use]
pub fn classify_count(count: &TallyCount, table: &AssetTable, ctx: Context) -> Class {
    if count.font {
        return Class::Text;
    }
    classify_pixel(
        &Pixel {
            layer: count.layer,
            bpp: count.bpp,
            asset: bound(table, count.asset, count.tile),
            pose: count.pose.map(|(fighter, _)| fighter),
        },
        ctx,
    )
}

/// The class of a thing a frame shows without art, and its stable name: `pose:<fighter|effect>:
/// <missing|partial>:<key>`, `tile:<asset id>#<tile>` or `char:<obj|bg>:<bits>bpp:<key>`.
#[must_use]
pub fn classify_item(key: &ItemKey, table: &AssetTable, ctx: Context) -> (Class, String) {
    match *key {
        ItemKey::Pose { key, fighter, fate } => {
            let class = classify_pixel(
                &Pixel { layer: TallyLayer::Obj, bpp: 4, asset: None, pose: Some(fighter) },
                ctx,
            );
            let who = if fighter { "fighter" } else { "effect" };
            let fate = match fate {
                ssd_hd::Fate::Art => "art",
                ssd_hd::Fate::Missing => "missing",
                ssd_hd::Fate::Partial => "partial",
            };
            (class, format!("pose:{who}:{fate}:{key:016x}"))
        }
        ItemKey::Tile { asset, tile, object } => {
            let layer = if object { TallyLayer::Obj } else { TallyLayer::Bg };
            let class = classify_pixel(
                &Pixel { layer, bpp: 0, asset: bound(table, Some(asset), tile), pose: None },
                ctx,
            );
            (class, format!("tile:{}#{tile}", table.id(asset)))
        }
        ItemKey::Char { object, bpp, key } => {
            let layer = if object { TallyLayer::Obj } else { TallyLayer::Bg };
            let class = classify_pixel(&Pixel { layer, bpp, asset: None, pose: None }, ctx);
            (class, format!("char:{}:{bpp}bpp:{key:016x}", if object { "obj" } else { "bg" }))
        }
    }
}

/// Where something was first seen: the part of the tour and the frame in it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Where {
    pub part: String,
    pub frame: u64,
}

/// What the tour counted of one class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassTotals {
    /// Visible pixels.
    pub pixels: u64,
    /// Of those, the ones the art draws.
    pub art: u64,
    /// Pixels of poses the art index lacks and of poses it cannot place (a fade-in, a sprite cut
    /// by the screen's edge).
    pub missing: u64,
    pub partial: u64,
}

impl ClassTotals {
    /// The share of the pixels the art draws, in percent (100 when there are none).
    #[must_use]
    pub fn percent(&self) -> f64 {
        if self.pixels == 0 { 100.0 } else { self.art as f64 * 100.0 / self.pixels as f64 }
    }

    fn add(&mut self, other: &Self) {
        self.pixels += other.pixels;
        self.art += other.art;
        self.missing += other.missing;
        self.partial += other.partial;
    }
}

/// A thing shown without art: what it is, how much of the screen it took, where it was first seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTotals {
    pub class: Class,
    /// Its stable name (see [`classify_item`]).
    pub id: String,
    pub pixels: u64,
    /// Frames it was in.
    pub frames: u64,
    pub first: Where,
    /// Its box in the frame at the first sighting (`x0, y0, x1, y1`).
    pub bbox: (u16, u16, u16, u16),
}

/// What a part of the tour (or all of them) counted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    /// Frames counted, and frames the renderer refused.
    pub frames: u64,
    pub refused: u64,
    /// Frames drawn as the field (the map beyond the window read through the lookup), as a flight
    /// (the Mode 7 plane) and as a picture (black beside the original's 256 pixels).
    #[serde(default)]
    pub field_frames: u64,
    #[serde(default)]
    pub flight_frames: u64,
    #[serde(default)]
    pub picture_frames: u64,
    pub classes: BTreeMap<Class, ClassTotals>,
    /// The things shown without art, by `<class>|<id>`.
    pub items: BTreeMap<String, ItemTotals>,
}

impl Totals {
    /// Adds one frame's tally; returns the keys of the items it showed for the first time in
    /// these totals (so a caller can keep a picture of them).
    pub fn add_frame(
        &mut self,
        tally: &FrameTally,
        ctx: Context,
        table: &AssetTable,
        at: &Where,
    ) -> Vec<String> {
        self.frames += 1;
        for count in &tally.counts {
            let class = classify_count(count, table, ctx);
            let totals = self.classes.entry(class).or_default();
            totals.pixels += count.pixels;
            if count.art {
                totals.art += count.pixels;
            }
            match count.pose {
                Some((_, ssd_hd::Fate::Missing)) => totals.missing += count.pixels,
                Some((_, ssd_hd::Fate::Partial)) => totals.partial += count.pixels,
                _ => {}
            }
        }
        let mut fresh = Vec::new();
        for item in &tally.items {
            let (class, id) = classify_item(&item.key, table, ctx);
            let key = format!("{}|{id}", class.key());
            match self.items.get_mut(&key) {
                Some(held) => {
                    held.pixels += item.pixels;
                    held.frames += 1;
                }
                None => {
                    self.items.insert(
                        key.clone(),
                        ItemTotals {
                            class,
                            id,
                            pixels: item.pixels,
                            frames: 1,
                            first: at.clone(),
                            bbox: item.bbox,
                        },
                    );
                    fresh.push(key);
                }
            }
        }
        fresh
    }

    /// Adds what `other` counted (a part that comes later in the tour).
    pub fn merge(&mut self, other: &Self) {
        self.frames += other.frames;
        self.refused += other.refused;
        self.field_frames += other.field_frames;
        self.flight_frames += other.flight_frames;
        self.picture_frames += other.picture_frames;
        for (class, totals) in &other.classes {
            self.classes.entry(*class).or_default().add(totals);
        }
        for (key, item) in &other.items {
            match self.items.get_mut(key) {
                Some(held) => {
                    held.pixels += item.pixels;
                    held.frames += item.frames;
                }
                None => {
                    self.items.insert(key.clone(), item.clone());
                }
            }
        }
    }

    /// The totals of `class` (zero when none was seen).
    #[must_use]
    pub fn class(&self, class: Class) -> ClassTotals {
        self.classes.get(&class).copied().unwrap_or_default()
    }

    /// The items with no art of `class`, the ones that took the most pixels first.
    #[must_use]
    pub fn unbound(&self, class: Class) -> Vec<(&String, &ItemTotals)> {
        let mut list: Vec<_> = self.items.iter().filter(|(_, item)| item.class == class).collect();
        list.sort_by(|a, b| b.1.pixels.cmp(&a.1.pixels).then(a.0.cmp(b.0)));
        list
    }
}

/// The config of the tour: which classes are done, and the share of their pixels the art must
/// draw (`tools/hd-tour.toml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Config {
    /// The least coverage, in percent, of a class marked done.
    #[serde(default = "default_threshold")]
    pub threshold: f64,
    /// The classes ([`Class::key`]) the art covers: the tour fails when one of them is below
    /// the threshold.
    #[serde(default)]
    pub done: Vec<String>,
}

fn default_threshold() -> f64 {
    99.5
}

impl Default for Config {
    fn default() -> Self {
        Self { threshold: default_threshold(), done: Vec::new() }
    }
}

impl Config {
    /// Reads the config.
    ///
    /// # Errors
    ///
    /// The text is not the config, or names a class that does not exist.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let config: Self = toml::from_str(text).context("reading the tour config")?;
        for name in &config.done {
            anyhow::ensure!(
                Class::parse(name).is_some(),
                "the tour config names no class `{name}`"
            );
        }
        anyhow::ensure!((0.0..=100.0).contains(&config.threshold), "the threshold is a percentage");
        Ok(config)
    }

    /// Whether `class` is marked done.
    #[must_use]
    pub fn is_done(&self, class: Class) -> bool {
        self.done.iter().any(|name| name == class.key())
    }

    /// The classes marked done whose coverage in `totals` is below the threshold, among the
    /// `selected` ones (a class that was not seen is not judged).
    #[must_use]
    pub fn failures(&self, totals: &Totals, selected: &[Class]) -> Vec<Class> {
        CLASSES
            .iter()
            .copied()
            .filter(|&class| selected.contains(&class) && self.is_done(class))
            .filter(|&class| {
                let counted = totals.class(class);
                counted.pixels > 0 && counted.percent() < self.threshold
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TallyLayer::{Bg, Obj, Plane};
    use ssd_hd::{Fate, TallyItem};

    fn table() -> AssetTable {
        AssetTable::of_ids(&[
            "portrait/16:8000/4bpp",
            "ui/11:8036/2bpp",
            "field/08:8000/4bpp",
            "sprite/0F:81F7/4bpp",
            "picture/09:83FA/4bpp",
            "picture/11:C000/2bpp",
            "title/00:C7DA/4bpp",
            "fighter/1C:94FC/4bpp",
        ])
    }

    fn count(
        layer: TallyLayer,
        asset: Option<u32>,
        tile: u32,
        art: bool,
        pixels: u64,
    ) -> TallyCount {
        TallyCount { layer, bpp: 4, asset, tile, pose: None, art, font: false, pixels }
    }

    const FIELD: Context = Context { battle: false, animation: false };
    const COMMAND: Context = Context { battle: true, animation: false };
    const ANIMATION: Context = Context { battle: true, animation: true };

    #[test]
    fn assets_are_told_apart_by_the_id_of_their_stream() {
        let table = table();
        let info = table.get(4).unwrap();
        assert_eq!((info.kind, info.bank, info.columns), (Kind::Picture, 0x09, 16));
        assert_eq!(table.id(1), "ui/11:8036/2bpp");
        assert_eq!(table.id(99), "?");
        assert_eq!(table.get(0).unwrap().kind, Kind::Portrait);
    }

    #[test]
    fn the_scene_comes_from_the_game_mode() {
        let mut wram = vec![0_u8; 0x30];
        assert_eq!(Context::of_wram(&wram), FIELD);
        wram[GAME_MODE] = 0x82;
        assert_eq!(Context::of_wram(&wram), COMMAND);
        wram[GAME_MODE] = 0x04;
        assert_eq!(Context::of_wram(&wram), ANIMATION);
        wram[GAME_MODE] = 0x0C;
        assert_eq!(Context::of_wram(&wram), ANIMATION);
        assert_eq!(Context::of_wram(&[]), FIELD);
    }

    #[test]
    fn a_pixel_is_classed_by_its_asset_and_the_scene() {
        let table = table();
        let class = |layer, asset, tile, ctx| {
            classify_count(&count(layer, asset, tile, false, 1), &table, ctx)
        };
        assert_eq!(class(Bg, Some(0), 0, FIELD), Class::Portraits);
        assert_eq!(class(Obj, Some(0), 0, ANIMATION), Class::Portraits);
        assert_eq!(class(Bg, Some(1), 40, FIELD), Class::Text);
        assert_eq!(class(Bg, Some(1), 1, FIELD), Class::CardsUi, "a solid fill");
        assert_eq!(class(Bg, Some(1), 52, FIELD), Class::CardsUi, "a piece of a frame");
        assert_eq!(class(Bg, Some(1), 150, FIELD), Class::CardsUi);
        assert_eq!(class(Bg, Some(2), 0, FIELD), Class::Map);
        assert_eq!(class(Obj, Some(3), 0, FIELD), Class::FieldSprites);
        assert_eq!(class(Bg, Some(4), 0, FIELD), Class::Pictures);
        assert_eq!(class(Bg, Some(4), 0, COMMAND), Class::BattleBg);
        assert_eq!(class(Bg, Some(5), 0, ANIMATION), Class::BattleBg);
        assert_eq!(class(Bg, Some(6), 0, FIELD), Class::Pictures);
        assert_eq!(class(Bg, Some(7), 0, ANIMATION), Class::BattleBg);
        assert_eq!(class(Plane, None, 0, FIELD), Class::Map);
        assert_eq!(class(Plane, None, 0, ANIMATION), Class::BattleBg);
        assert_eq!(class(Obj, Some(7), 0, COMMAND), Class::CardsUi);
        assert_eq!(class(Obj, Some(7), 0, ANIMATION), Class::Effects);
    }

    #[test]
    fn pixels_the_hd_font_draws_are_text_whatever_they_are_bound_to() {
        let table = table();
        let mut drawn = count(TallyLayer::Bg, Some(1), 40, true, 8);
        drawn.font = true;
        assert_eq!(classify_count(&drawn, &table, FIELD), Class::Text);
        assert_eq!(classify_count(&drawn, &table, COMMAND), Class::Text);
        // Drawn by the font, so drawn from art
        let mut totals = Totals::default();
        let tally = FrameTally {
            counts: vec![drawn, count(TallyLayer::Bg, Some(1), 41, false, 2)],
            items: vec![],
            shapes: vec![],
            uses: vec![],
        };
        totals.add_frame(&tally, FIELD, &table, &Where::default());
        assert_eq!(
            totals.class(Class::Text),
            ClassTotals { pixels: 10, art: 8, ..ClassTotals::default() }
        );
    }

    #[test]
    fn poses_are_fighters_or_effects_in_a_battle_and_field_sprites_outside() {
        let table = table();
        let pose = |fighter, ctx| {
            let mut c = count(TallyLayer::Obj, None, 0, true, 1);
            c.pose = Some((fighter, Fate::Art));
            classify_count(&c, &table, ctx)
        };
        assert_eq!(pose(true, COMMAND), Class::Fighters);
        assert_eq!(pose(true, FIELD), Class::FieldSprites);
        assert_eq!(pose(false, ANIMATION), Class::Effects);
        assert_eq!(pose(false, COMMAND), Class::CardsUi);
        assert_eq!(pose(false, FIELD), Class::FieldSprites);
    }

    #[test]
    fn characters_bound_to_no_asset_are_classed_by_layer_and_scene() {
        let table = table();
        let char_item = |object, bpp, ctx| {
            classify_item(&ItemKey::Char { object, bpp, key: 0xAB }, &table, ctx)
        };
        assert_eq!(char_item(false, 2, FIELD).0, Class::Text);
        assert_eq!(char_item(false, 4, FIELD).0, Class::Map);
        assert_eq!(char_item(false, 2, COMMAND).0, Class::BattleBg);
        assert_eq!(char_item(true, 4, FIELD).0, Class::FieldSprites);
        assert_eq!(char_item(true, 4, ANIMATION).0, Class::Effects);
        assert_eq!(char_item(true, 4, FIELD).1, "char:obj:4bpp:00000000000000ab");
    }

    #[test]
    fn items_have_stable_names() {
        let table = table();
        let (class, id) =
            classify_item(&ItemKey::Tile { asset: 1, tile: 200, object: false }, &table, FIELD);
        assert_eq!((class, id.as_str()), (Class::CardsUi, "tile:ui/11:8036/2bpp#200"));
        let (class, _) =
            classify_item(&ItemKey::Tile { asset: 1, tile: 20, object: false }, &table, FIELD);
        assert_eq!(class, Class::Text);
        let (class, id) = classify_item(
            &ItemKey::Pose { key: 0x1234, fighter: true, fate: Fate::Missing },
            &table,
            COMMAND,
        );
        assert_eq!(
            (class, id.as_str()),
            (Class::Fighters, "pose:fighter:missing:0000000000001234")
        );
    }

    fn tally(counts: Vec<TallyCount>, items: Vec<TallyItem>) -> FrameTally {
        FrameTally { counts, items, shapes: vec![], uses: vec![] }
    }

    fn tile_item(tile: u32, pixels: u64) -> TallyItem {
        TallyItem {
            key: ItemKey::Tile { asset: 2, tile, object: false },
            pixels,
            bbox: (1, 2, 3, 4),
        }
    }

    #[test]
    fn a_frame_adds_its_pixels_to_the_classes_and_its_items_to_the_list() {
        let table = table();
        let at = Where { part: "map-00".to_owned(), frame: 7 };
        let mut totals = Totals::default();
        let first = tally(
            vec![
                count(TallyLayer::Bg, Some(0), 0, true, 90),
                count(TallyLayer::Bg, Some(0), 0, false, 10),
                count(TallyLayer::Bg, Some(2), 0, false, 500),
            ],
            vec![tile_item(5, 500)],
        );
        let fresh = totals.add_frame(&first, FIELD, &table, &at);
        assert_eq!(fresh, vec!["map|tile:field/08:8000/4bpp#5".to_owned()]);
        assert_eq!(
            totals.class(Class::Portraits),
            ClassTotals { pixels: 100, art: 90, ..ClassTotals::default() }
        );
        assert!((totals.class(Class::Portraits).percent() - 90.0).abs() < 1e-9);
        assert_eq!(totals.class(Class::Map).art, 0);
        // The same tile again is not new, and the first sighting stays
        let later = Where { part: "map-00".to_owned(), frame: 9 };
        let fresh = totals.add_frame(
            &tally(vec![], vec![tile_item(5, 20), tile_item(6, 3)]),
            FIELD,
            &table,
            &later,
        );
        assert_eq!(fresh, vec!["map|tile:field/08:8000/4bpp#6".to_owned()]);
        let item = &totals.items["map|tile:field/08:8000/4bpp#5"];
        assert_eq!((item.pixels, item.frames, item.first.frame), (520, 2, 7));
        assert_eq!(totals.frames, 2);
        let listed = totals.unbound(Class::Map);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].1.pixels, 520);
        assert_eq!(Totals::default().class(Class::Text).percent(), 100.0);
    }

    #[test]
    fn missing_and_partial_poses_are_counted_apart() {
        let table = table();
        let mut totals = Totals::default();
        let mut missing = count(TallyLayer::Obj, None, 0, false, 40);
        missing.pose = Some((true, Fate::Missing));
        let mut partial = count(TallyLayer::Obj, None, 0, false, 6);
        partial.pose = Some((true, Fate::Partial));
        let mut art = count(TallyLayer::Obj, None, 0, true, 954);
        art.pose = Some((true, Fate::Art));
        totals.add_frame(
            &tally(vec![missing, partial, art], vec![]),
            COMMAND,
            &table,
            &Where::default(),
        );
        let fighters = totals.class(Class::Fighters);
        assert_eq!(
            (fighters.pixels, fighters.art, fighters.missing, fighters.partial),
            (1000, 954, 40, 6)
        );
        assert!((fighters.percent() - 95.4).abs() < 1e-9);
    }

    #[test]
    fn totals_merge_in_order() {
        let table = table();
        let one = {
            let mut totals = Totals::default();
            totals.add_frame(
                &tally(vec![count(TallyLayer::Bg, Some(2), 0, false, 10)], vec![tile_item(1, 10)]),
                FIELD,
                &table,
                &Where { part: "a".to_owned(), frame: 1 },
            );
            totals.refused = 2;
            totals
        };
        let two = {
            let mut totals = Totals::default();
            totals.add_frame(
                &tally(
                    vec![count(TallyLayer::Bg, Some(2), 0, true, 5)],
                    vec![tile_item(1, 7), tile_item(2, 1)],
                ),
                FIELD,
                &table,
                &Where { part: "b".to_owned(), frame: 3 },
            );
            totals
        };
        let mut all = Totals::default();
        all.merge(&one);
        all.merge(&two);
        assert_eq!((all.frames, all.refused), (2, 2));
        assert_eq!(
            all.class(Class::Map),
            ClassTotals { pixels: 15, art: 5, ..ClassTotals::default() }
        );
        let held = &all.items["map|tile:field/08:8000/4bpp#1"];
        assert_eq!((held.pixels, held.frames, held.first.part.as_str()), (17, 2, "a"));
        assert_eq!(all.items["map|tile:field/08:8000/4bpp#2"].first.part, "b");
    }

    #[test]
    fn totals_survive_their_json() {
        let table = table();
        let mut totals = Totals::default();
        totals.add_frame(
            &tally(vec![count(TallyLayer::Bg, Some(2), 0, false, 10)], vec![tile_item(1, 10)]),
            FIELD,
            &table,
            &Where { part: "a".to_owned(), frame: 1 },
        );
        let text = serde_json::to_string(&totals).unwrap();
        assert!(text.contains("\"map\""), "{text}");
        let back: Totals = serde_json::from_str(&text).unwrap();
        assert_eq!(back, totals);
    }

    #[test]
    fn the_config_names_the_classes_that_are_done() {
        let config =
            Config::parse("threshold = 99.5\ndone = [\"portraits\", \"fighters\"]\n").unwrap();
        assert!(config.is_done(Class::Portraits) && !config.is_done(Class::Map));
        assert!(
            Config::parse("done = [\"nonsense\"]").unwrap_err().to_string().contains("nonsense")
        );
        assert!(Config::parse("threshold = 120").is_err());
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn a_done_class_below_the_threshold_fails_only_when_it_was_seen_and_selected() {
        let config = Config { threshold: 99.5, done: vec!["portraits".into(), "fighters".into()] };
        let mut totals = Totals::default();
        totals.classes.insert(
            Class::Portraits,
            ClassTotals { pixels: 1000, art: 996, ..ClassTotals::default() },
        );
        totals.classes.insert(
            Class::Fighters,
            ClassTotals { pixels: 1000, art: 994, ..ClassTotals::default() },
        );
        totals
            .classes
            .insert(Class::Map, ClassTotals { pixels: 1000, art: 0, ..ClassTotals::default() });
        assert_eq!(config.failures(&totals, &CLASSES), vec![Class::Fighters]);
        assert_eq!(config.failures(&totals, &[Class::Portraits, Class::Map]), Vec::new());
        // Not seen: nothing to judge
        assert_eq!(config.failures(&Totals::default(), &CLASSES), Vec::new());
    }

    #[test]
    fn classes_have_names_that_parse() {
        for class in CLASSES {
            assert_eq!(Class::parse(class.key()), Some(class));
        }
        assert_eq!(Class::parse("fonts"), None);
    }
}
