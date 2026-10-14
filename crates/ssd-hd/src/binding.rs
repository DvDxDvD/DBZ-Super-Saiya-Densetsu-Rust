//! Tile binding: which pack tile each character of VRAM is, for one state of VRAM.
//!
//! A character (a 4-bit tile of 16 words, a 2-bit one of 8) binds when its key is unique in the
//! index, or when it sits in a run of at least two characters that match the tiles of one asset
//! in order: the run rule. The same tile art can be in several streams, and a single tile of it
//! cannot tell which; a run of the stream's tiles can, because the game uploads whole streams
//! (portraits in two runs of eight). A blank character binds to nothing: it draws no pixel.

use crate::pack::{Index, Pack, TileRef, word_key};
use std::sync::Arc;

/// How far the run rule looks each way from a character. Two would decide; more only breaks ties
/// between candidates, and the cap keeps a long run of one repeated tile from costing a square.
const RUN_LIMIT: usize = 32;

/// The binding of one VRAM state: the pack tile of every 4-bit and every 2-bit character.
pub struct Binder<'a> {
    index: IndexRef<'a>,
    /// The VRAM the binding is for, when it has been made.
    vram: Option<Vec<u16>>,
    four: Vec<Option<TileRef>>,
    two: Vec<Option<TileRef>>,
    /// Counts the bindings made, so an answer taken from one can be told from another's.
    epoch: u32,
}

/// The index a binder reads: borrowed, or the one of a pack it shares.
enum IndexRef<'a> {
    Borrowed(&'a Index),
    Pack(Arc<Pack>),
}

impl IndexRef<'_> {
    fn get(&self) -> &Index {
        match self {
            Self::Borrowed(index) => index,
            Self::Pack(pack) => &pack.index,
        }
    }
}

impl<'a> Binder<'a> {
    /// A binder for the pack `index`; nothing is bound until [`Binder::update`].
    #[must_use]
    pub fn new(index: &'a Index) -> Self {
        Self {
            index: IndexRef::Borrowed(index),
            vram: None,
            four: Vec::new(),
            two: Vec::new(),
            epoch: 0,
        }
    }

    /// A binder that keeps the pack it binds to.
    #[must_use]
    pub fn of_pack(pack: Arc<Pack>) -> Binder<'static> {
        Binder {
            index: IndexRef::Pack(pack),
            vram: None,
            four: Vec::new(),
            two: Vec::new(),
            epoch: 0,
        }
    }

    /// Binds the characters of `vram`. Nothing is recomputed when `vram` is the VRAM already bound.
    pub fn update(&mut self, vram: &[u16]) {
        if self.vram.as_deref() == Some(vram) {
            return;
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.four = bind(self.index.get(), vram, 4);
        self.two = bind(self.index.get(), vram, 2);
        self.vram = Some(vram.to_vec());
    }

    /// A number that changes whenever the binding does.
    #[must_use]
    pub(crate) fn epoch(&self) -> u32 {
        self.epoch
    }

    /// The pack tile of the character of `bpp` bits (2 or 4) at `address`, a VRAM word address.
    /// `None` when the character is not in the pack, or no binding was made.
    #[must_use]
    pub fn tile(&self, bpp: u8, address: u16) -> Option<TileRef> {
        let (words, table) = if bpp == 4 { (16, &self.four) } else { (8, &self.two) };
        table.get(usize::from(address) / words).copied().flatten()
    }
}

/// The binding of the characters of `bpp` bits in `vram`, slot by slot.
fn bind(index: &Index, vram: &[u16], bpp: u8) -> Vec<Option<TileRef>> {
    let words = if bpp == 4 { 16 } else { 8 };
    let keys: Vec<Option<u64>> = vram
        .chunks_exact(words)
        .map(|chars| chars.iter().any(|&word| word != 0).then(|| word_key(chars)))
        .collect();
    let mut bound = vec![None; keys.len()];
    for (slot, key) in keys.iter().enumerate() {
        let Some(key) = *key else { continue };
        bound[slot] = match index.candidates(bpp, key) {
            [] => None,
            [only] => Some(*only),
            // The same art at several places of one asset: identical bytes draw identically, so
            // the first place is as good as any
            [first, rest @ ..] if rest.iter().all(|candidate| candidate.asset == first.asset) => {
                Some(*first)
            }
            candidates => {
                let mut best: Option<(usize, TileRef)> = None;
                for &candidate in candidates {
                    let run =
                        run_length(&keys, slot, &index.asset(candidate.asset).keys, candidate.tile);
                    if run >= 2 && best.is_none_or(|(longest, _)| run > longest) {
                        best = Some((run, candidate));
                    }
                }
                best.map(|(_, candidate)| candidate)
            }
        };
    }
    bound
}

/// The number of consecutive characters through `slot` that match `asset_keys` in order, from
/// the tile `tile` on (both ways, up to [`RUN_LIMIT`] each).
fn run_length(keys: &[Option<u64>], slot: usize, asset_keys: &[u64], tile: u32) -> usize {
    let tile = tile as usize;
    let matches = |slot: usize, tile: usize| match (keys.get(slot), asset_keys.get(tile)) {
        (Some(Some(key)), Some(asset_key)) => key == asset_key,
        _ => false,
    };
    let mut length = 1;
    let forward = (1..=RUN_LIMIT).take_while(|&step| matches(slot + step, tile + step)).count();
    let backward = (1..=RUN_LIMIT.min(slot).min(tile))
        .take_while(|&step| matches(slot - step, tile - step))
        .count();
    length += forward + backward;
    length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{Asset, Mode, word_key};

    fn asset(id: &str, keys: Vec<u64>) -> Asset {
        Asset {
            id: id.to_owned(),
            bpp: 4,
            mode: Mode::Indexed,
            columns: 4,
            keys,
            art: format!("base/{id}.png"),
        }
    }

    /// A 4-bit character whose first word is `seed`, the others zero (never blank).
    fn character(seed: u16) -> [u16; 16] {
        let mut chars = [0_u16; 16];
        chars[0] = seed;
        chars
    }

    fn vram_of(characters: &[[u16; 16]]) -> Vec<u16> {
        let mut vram = vec![0_u16; 32768];
        for (slot, chars) in characters.iter().enumerate() {
            vram[slot * 16..slot * 16 + 16].copy_from_slice(chars);
        }
        vram
    }

    #[test]
    fn a_unique_key_binds_alone_and_a_blank_character_binds_to_nothing() {
        let index = Index::new(vec![asset("a", vec![word_key(&character(5))])]);
        let vram = vram_of(&[[0; 16], character(5), character(6)]);
        let mut binder = Binder::new(&index);
        binder.update(&vram);
        assert_eq!(binder.tile(4, 16), Some(TileRef { asset: 0, tile: 0 }));
        assert_eq!(binder.tile(4, 0), None, "blank");
        assert_eq!(binder.tile(4, 32), None, "not in the pack");
        assert_eq!(binder.tile(2, 16), None, "no 2-bit asset holds it");
    }

    #[test]
    fn an_ambiguous_key_binds_only_in_a_run_of_its_asset_and_the_longest_run_wins() {
        // Asset a is the pair (1, 2); asset b is (9, 1, 2, 3). The shared pair is ambiguous.
        let index = Index::new(vec![
            asset("a", vec![word_key(&character(1)), word_key(&character(2))]),
            asset(
                "b",
                vec![
                    word_key(&character(9)),
                    word_key(&character(1)),
                    word_key(&character(2)),
                    word_key(&character(3)),
                ],
            ),
        ]);
        // VRAM holds 1, 2, 3: the run 1-2-3 is asset b's tiles 1 to 3, a run of three, while
        // asset a matches only two of them
        let mut binder = Binder::new(&index);
        binder.update(&vram_of(&[character(1), character(2), character(3)]));
        assert_eq!(binder.tile(4, 0), Some(TileRef { asset: 1, tile: 1 }), "the longest run wins");
        assert_eq!(binder.tile(4, 16), Some(TileRef { asset: 1, tile: 2 }));
        assert_eq!(binder.tile(4, 32), Some(TileRef { asset: 1, tile: 3 }), "unique");

        // One ambiguous tile on its own, with nothing after it that matches, binds to nothing
        binder.update(&vram_of(&[character(1), character(7)]));
        assert_eq!(binder.tile(4, 0), None);
    }

    #[test]
    fn a_tile_repeated_within_one_asset_binds_alone_to_its_first_place() {
        // Tile 7 is at two places of asset a: one character on its own still binds (the art of
        // the two places is the same, so either place does)
        let index = Index::new(vec![asset(
            "a",
            vec![word_key(&character(7)), word_key(&character(8)), word_key(&character(7))],
        )]);
        let mut binder = Binder::new(&index);
        binder.update(&vram_of(&[character(7)]));
        assert_eq!(binder.tile(4, 0), Some(TileRef { asset: 0, tile: 0 }));
    }

    #[test]
    fn the_binding_follows_the_vram_it_was_made_for() {
        let index = Index::new(vec![asset("a", vec![word_key(&character(4))])]);
        let mut binder = Binder::new(&index);
        binder.update(&vram_of(&[character(4)]));
        assert!(binder.tile(4, 0).is_some());
        binder.update(&vram_of(&[character(8)]));
        assert_eq!(binder.tile(4, 0), None, "rebound when VRAM changed");
    }
}
