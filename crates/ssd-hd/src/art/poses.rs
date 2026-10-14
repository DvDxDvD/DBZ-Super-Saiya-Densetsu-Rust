//! Finding the battle poses of a frame: the sprites of the object layer, decoded to palette
//! indexes and assembled into poses, keyed exactly as the capture of the HD art keyed them (so a
//! pose of the frame finds the art made from the same pose).
//!
//! * A fighter is the sprites drawn with OAM palette 1 (the party) or 2 (the enemy); its key is
//!   the first 16 hex digits of the SHA-1 of the trimmed palette-index array (row-major bytes)
//!   followed by the bytes `height, width`. The key is taken over the whole palette group, and
//!   over its clusters of pieces (at most 12 pixels apart) when the group holds debris or sparks
//!   as well.
//! * An effect is a cluster of the sprites of the other palettes (pieces within 4 pixels of each
//!   other, above line 160); its key is the same hash over the trimmed RGBA array, whose colours
//!   are those of each pixel's own scanline (the HDMA gradients change them).
//!
//! * A field group (the party, NPCs, ships and objects of the field) is keyed like a fighter, over
//!   its palette indexes: the frame's visible sprites in OAM order are split into runs of
//!   consecutive entries of one OAM palette, and each run into sets of sprites whose boxes are
//!   within 2 pixels of each other ([`field_groups`]).
//!
//! The decoding of the sprites follows the capture, not the PPU: whole sprites (all their pixels,
//! the ones off the screen too), as the sprite sizes of the frame say.

use sha1_smol::Sha1;
use ssd_oracle::FrameLog;

/// One decoded sprite of the object layer.
pub(crate) struct Sprite {
    pub idx: u8,
    /// The position of the sprite's top-left pixel in OAM terms: x, and y as OAM says it
    /// (a pose's rows start one below).
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub pal: u8,
    /// OAM priority bits.
    pub priority: u8,
    pub pix: Vec<u8>,
}

fn sizes(index: u8) -> ((usize, usize), (usize, usize)) {
    match index {
        0 => ((8, 8), (16, 16)),
        1 => ((8, 8), (32, 32)),
        2 => ((8, 8), (64, 64)),
        3 => ((16, 16), (32, 32)),
        4 => ((16, 16), (64, 64)),
        5 => ((32, 32), (64, 64)),
        6 => ((16, 32), (32, 64)),
        _ => ((16, 32), (32, 32)),
    }
}

/// The 8-bit colour of a 15-bit one.
pub(crate) fn rgb555(value: u16) -> [u8; 3] {
    let channel = |x: u16| {
        let byte = ((x & 31) << 3) as u8;
        byte | byte >> 5
    };
    [channel(value), channel(value >> 5), channel(value >> 10)]
}

/// Where the sprites are looked for, in the window's own coordinates (x = 0 is the left edge of
/// the 256 pixels the game draws): the original window, or a wide view of the field, which also
/// has the margins on both sides and the objects the original drops.
#[derive(Debug, Clone)]
pub(crate) struct View {
    /// The columns the frame shows: `left..right`, `0..256` for the window alone.
    pub left: i32,
    pub right: i32,
    /// The frame's x of the window's left edge (0 for the window alone).
    pub window_x: i32,
    /// The parts of objects the original drops, placed in the frame's OAM.
    pub extras: Vec<ExtraSprite>,
}

impl View {
    /// The original window alone.
    pub(crate) fn window() -> Self {
        Self::window_at(0)
    }

    /// The original window alone, `window_x` pixels from the left of the frame.
    pub(crate) fn window_at(window_x: i32) -> Self {
        Self { left: 0, right: 256, window_x, extras: Vec::new() }
    }

    /// Whether frame column `x` is outside the original window.
    pub(crate) fn in_margin(&self, x: i32) -> bool {
        x < self.window_x || x >= self.window_x + 256
    }
}

/// A part of a map object the original does not draw, which a wide view draws in an entry that
/// the original's OAM leaves free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExtraSprite {
    /// The OAM entry the part is drawn with (the same in every chunk of the frame).
    pub idx: u8,
    /// X of the part's left edge in the window's coordinates, and the rest of the entry.
    pub x: i32,
    pub y: u8,
    pub tile: u8,
    pub attributes: u8,
    pub large: bool,
    /// The entry of the original's OAM the part follows in the order the original's loop writes
    /// the objects (see `ssd_hd::world::drop_anchors`), or `None` when that is not known: the part
    /// then has the place of its own entry.
    pub after: Option<i32>,
    /// The part's place among the extras of the frame.
    pub order: u32,
}

/// One entry to decode: the OAM's or an extra part's.
struct Raw {
    idx: u8,
    x: i32,
    y: u8,
    tile: u16,
    attr: u8,
    large: bool,
    /// The place in the order of the sprites (see [`sprites_in`]).
    key: (i32, u8, u32),
}

/// The sprites of the frame `log` describes (its OAM and VRAM as they stood at the start), those
/// that touch the original window.
#[cfg(test)]
pub(crate) fn sprites(log: &FrameLog) -> Vec<Sprite> {
    sprites_in(log, &View::window())
}

/// The sprites of the frame `log` describes that touch `view`, in the order of the original's
/// OAM: the extra parts of a wide view follow the entry of the object they belong to, so a group
/// of the field is made of the same sprites in the same order whatever side of the window's edge
/// the pieces of its object lie.
pub(crate) fn sprites_in(log: &FrameLog, view: &View) -> Vec<Sprite> {
    // (the entry the sprite follows, 0 for an entry of the OAM and 1 for an extra part, the order)
    let mut found: Vec<((i32, u8, u32), Sprite)> = Vec::new();
    let mut entries: Vec<Raw> = Vec::new();
    for index in 0..128_usize {
        let low = &log.oam_low[index * 4..index * 4 + 4];
        let high = log.oam_high[index >> 2] >> (2 * (index & 3));
        let x9 = u16::from(low[0]) | (u16::from(high & 1) << 8);
        let attr = low[3];
        let tile = u16::from(low[2]) | (u16::from(attr & 1) << 8);
        let sx = if x9 >= 256 { i32::from(x9) - 512 } else { i32::from(x9) };
        entries.push(Raw {
            idx: index as u8,
            x: sx,
            y: low[1],
            tile,
            attr,
            large: high & 2 != 0,
            key: (index as i32, 0, 0),
        });
    }
    for extra in &view.extras {
        let attr = extra.attributes;
        let tile = u16::from(extra.tile) | (u16::from(attr & 1) << 8);
        let key = match extra.after {
            Some(after) => (after, 1, extra.order),
            None => (i32::from(extra.idx), 0, 0),
        };
        entries.push(Raw {
            idx: extra.idx,
            x: extra.x,
            y: extra.y,
            tile,
            attr,
            large: extra.large,
            key,
        });
    }
    for Raw { idx, x: sx, y, tile, attr, large, key } in entries {
        let line = u16::from(y.wrapping_add(1));
        let entry = log
            .entries
            .iter()
            .find(|e| e.sprites.eval_line == line && !e.sprites.blank)
            .or_else(|| log.entries.iter().find(|e| !e.sprites.blank));
        let Some(entry) = entry else { continue };
        let sp = &entry.sprites;
        let (small, big) = sizes(sp.fetch_size);
        let (w, h) = if large { big } else { small };
        if sx + w as i32 <= view.left || sx >= view.right {
            continue;
        }
        if y >= 224 && i32::from(y) + h as i32 <= 256 {
            continue;
        }
        let sy = if y >= 224 { i32::from(y) - 256 } else { i32::from(y) };
        let (x_flip, y_flip) = (attr & 0x40 != 0, attr & 0x80 != 0);
        let mut pix = vec![0_u8; w * h];
        for ty in 0..h / 8 {
            for tx in 0..w / 8 {
                let sx_tile = if x_flip { w / 8 - 1 - tx } else { tx } as u16;
                let sy_tile = if y_flip { h / 8 - 1 - ty } else { ty } as u16;
                let mut number = tile;
                number = (number & !0xF) | (number.wrapping_add(sx_tile) & 0xF);
                number = (number & !0xF0) | (number.wrapping_add(sy_tile << 4) & 0xF0);
                let base = sp
                    .fetch_base
                    .wrapping_add(u16::from(number & 0x100 != 0) * (256 * 16 + sp.fetch_gap));
                let address = usize::from(base.wrapping_add((number & 0xFF) * 16) & 0x7FFF);
                let data = &log.vram[address..address + 16];
                for row in 0..8_usize {
                    for col in 0..8_usize {
                        let bit = 7 - col;
                        let mut color = 0_u8;
                        for plane in 0..2_usize {
                            let word = data[row + 8 * plane];
                            color |= u8::from((word >> bit) & 1 != 0) << (2 * plane);
                            color |= u8::from((word >> (bit + 8)) & 1 != 0) << (2 * plane + 1);
                        }
                        let fx = tx * 8 + if x_flip { 7 - col } else { col };
                        let fy = ty * 8 + if y_flip { 7 - row } else { row };
                        pix[fy * w + fx] = color;
                    }
                }
            }
        }
        if pix.iter().all(|&c| c == 0) {
            continue;
        }
        found.push((
            key,
            Sprite {
                idx,
                x: sx,
                y: sy,
                w,
                h,
                pal: (attr >> 1) & 7,
                priority: (attr >> 4) & 3,
                pix,
            },
        ));
    }
    found.sort_by_key(|(key, _)| *key);
    found.into_iter().map(|(_, sprite)| sprite).collect()
}

/// What a pose is made of: its sprites (a bit for each OAM entry), its trimmed array and where it
/// sits.
#[derive(Clone)]
pub(crate) struct Pose {
    /// The key of the pose's pixels (see the module documentation).
    pub key: u64,
    pub fighter: bool,
    /// A sprite group of the field (see [`field_groups`]): neither a fighter nor an effect.
    pub field: bool,
    /// A field group with a sprite at the screen's edge: pieces that are off the screen are not in
    /// it, so it may be only part of a pose of the index.
    pub cut: bool,
    /// Bit `n`: OAM entry `n` is one of the pose's pieces.
    pub sprites: u128,
    /// The lowest OAM index of the pose's pieces and the OAM priority bits of its frontmost piece.
    pub first: u8,
    pub priority: u8,
    /// The palette the pieces of a fighter are drawn with (the first piece's for an effect).
    pub palette: u8,
    pub w: usize,
    pub h: usize,
    /// The trimmed palette-index array, row by row (0 transparent).
    pub arr: Vec<u8>,
    /// The trimmed colours of an effect (RGBA, row by row; empty for a fighter's or a field group).
    pub rgba: Vec<u8>,
    /// Where the array's top-left pixel is, in the capture's terms: x of the screen, and the row
    /// the capture counts (one below the OAM `y` of the topmost piece, see [`ROW_OF_CAPTURE`]).
    pub org: (i32, i32),
    /// An effect of one dark flat colour in the shape of an ellipse: a fighter's ground shadow
    /// (see [`is_shadow`]).
    pub shadow: bool,
    /// Sprites of the pose that are a shadow of their own, drawn as one ([`shadow_sprites`]):
    /// the pose's art is not drawn over their pixels.
    pub excluded: u128,
    /// The colours of the palette the pose is drawn with, when the art that matches it was made
    /// for another palette: the art is drawn recoloured to them ([`super::recolour`]).
    pub recolour: Option<super::recolour::Variant>,
    /// A fighter drawn as BG tiles (see `ArtState::bg_fighters`): `org` is in frame coordinates,
    /// `palette` is the BG palette number, the pose has no sprites.
    pub bg: bool,
}

impl Pose {
    /// The key the pose would have if the game drew it facing the other way: the key of its array
    /// mirrored left to right (a fighter's or a field group's; an effect's key is over colours and
    /// has no mirror here). The game flips a sprite with the hardware's flip bits, so the art made
    /// for one facing is the art of the other, drawn mirrored.
    pub(crate) fn mirror_key(&self) -> Option<u64> {
        if !self.rgba.is_empty() || self.w == 0 {
            return None;
        }
        let mut flipped = Vec::with_capacity(self.arr.len());
        for row in self.arr.chunks(self.w) {
            flipped.extend(row.iter().rev());
        }
        Some(digest(&flipped, [self.h as u8, self.w as u8]))
    }
}

/// The frame row of the first row of a pose's array, less the `y` of [`Pose::org`]: the capture
/// counts a sprite's rows from `y + 1`, the PPU draws row 0 of a sprite at the row `y`.
pub(crate) const ROW_OF_CAPTURE: i32 = -1;

fn digest(data: &[u8], tail: [u8; 2]) -> u64 {
    let mut hash = Sha1::new();
    hash.update(data);
    hash.update(&tail);
    u64::from_be_bytes(hash.digest().bytes()[..8].try_into().expect("eight bytes"))
}

/// The CGRAM as it stands at each entry of the log, to colour the sprites' rows.
pub(crate) struct Palettes {
    states: Vec<Vec<u16>>,
}

impl Palettes {
    pub(crate) fn of(log: &FrameLog) -> Self {
        let mut states: Vec<Vec<u16>> = Vec::with_capacity(log.entries.len());
        let mut current = log.cgram_start.clone();
        let mut writes = 0;
        for index in 0..log.entries.len() {
            while writes < log.cgram_writes.len()
                && usize::from(log.cgram_writes[writes].entry) <= index
            {
                let write = &log.cgram_writes[writes];
                current[usize::from(write.index)] = write.value;
                writes += 1;
            }
            states.push(current.clone());
        }
        Self { states }
    }

    /// The colour of pixel `index` of palette `pal` on the 1-based scanline `scan`.
    fn color(&self, log: &FrameLog, scan: i32, pal: u8, index: u8) -> [u8; 3] {
        let at = log.entries.iter().position(|e| i32::from(e.scanline) == scan).unwrap_or(0);
        let state = &self.states[at.min(self.states.len() - 1)];
        rgb555(state[128 + usize::from(pal) * 16 + usize::from(index)])
    }
}

/// Clusters of sprites whose boxes are within `gap` pixels of each other, as lists of indexes into
/// `sprites`, in the order the capture found them.
fn components(sprites: &[&Sprite], gap: i32) -> Vec<Vec<usize>> {
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let n = sprites.len();
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (sprites[i], sprites[j]);
            if a.x - gap < b.x + b.w as i32
                && b.x - gap < a.x + a.w as i32
                && a.y - gap < b.y + b.h as i32
                && b.y - gap < a.y + a.h as i32
            {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                parent[ri] = rj;
            }
        }
    }
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        match groups.iter_mut().find(|(r, _)| *r == root) {
            Some((_, group)) => group.push(i),
            None => groups.push((root, vec![i])),
        }
    }
    groups.into_iter().map(|(_, group)| group).collect()
}

/// Assembles `group` (a lower OAM index on top), trims it and keys it. A fighter is keyed over
/// its palette indexes, an effect over its colours (`colors` gives the colour of a row's pixel).
pub(super) fn assemble(
    group: &[&Sprite],
    fighter: bool,
    colors: &dyn Fn(&Sprite, usize, u8) -> [u8; 3],
) -> Option<Pose> {
    let x0 = group.iter().map(|s| s.x).min()?;
    let y0 = group.iter().map(|s| s.y).min()? + 1;
    let x1 = group.iter().map(|s| s.x + s.w as i32).max()?;
    let y1 = group.iter().map(|s| s.y + 1 + s.h as i32).max()?;
    let (gw, gh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut idx = vec![0_u8; gw * gh];
    let mut rgb = if fighter { Vec::new() } else { vec![0_u8; gw * gh * 3] };
    let mut order: Vec<&&Sprite> = group.iter().collect();
    order.sort_by_key(|s| std::cmp::Reverse(s.idx));
    for s in order {
        for yy in 0..s.h {
            for xx in 0..s.w {
                let color = s.pix[yy * s.w + xx];
                if color == 0 {
                    continue;
                }
                let at = ((s.y + 1 - y0) as usize + yy) * gw + (s.x - x0) as usize + xx;
                idx[at] = color;
                if !fighter {
                    rgb[at * 3..at * 3 + 3].copy_from_slice(&colors(s, yy, color));
                }
            }
        }
    }
    let (mut top, mut bottom, mut left, mut right) = (gh, 0, gw, 0);
    for y in 0..gh {
        for x in 0..gw {
            if idx[y * gw + x] != 0 {
                top = top.min(y);
                bottom = bottom.max(y + 1);
                left = left.min(x);
                right = right.max(x + 1);
            }
        }
    }
    if bottom == 0 {
        return None;
    }
    let (w, h) = (right - left, bottom - top);
    let mut arr = Vec::with_capacity(w * h);
    let mut rgba = Vec::with_capacity(if fighter { 0 } else { w * h * 4 });
    for y in top..bottom {
        for x in left..right {
            let color = idx[y * gw + x];
            arr.push(color);
            if !fighter {
                if color == 0 {
                    rgba.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    rgba.extend_from_slice(&rgb[(y * gw + x) * 3..][..3]);
                    rgba.push(255);
                }
            }
        }
    }
    let tail = [h as u8, w as u8];
    let key = if fighter { digest(&arr, tail) } else { digest(&rgba, tail) };
    let shadow = !fighter && flat_dark(&rgba) && ellipse(&arr, w, h);
    let mut sprites = 0_u128;
    for s in group {
        sprites |= 1 << s.idx;
    }
    Some(Pose {
        key,
        fighter,
        field: false,
        cut: false,
        sprites,
        first: group.iter().map(|s| s.idx).min().unwrap_or(0),
        priority: group.iter().map(|s| s.priority).max().unwrap_or(0),
        palette: group[0].pal,
        w,
        h,
        arr,
        rgba,
        org: (x0 + left as i32, y0 + top as i32),
        shadow,
        excluded: 0,
        recolour: None,
        bg: false,
    })
}

/// The pose of a fighter drawn as BG tiles: `arr` (`w` x `h`, the palette indexes of the BG pixels
/// of the cluster) with its top left pixel at the frame's `org` (x, and the row the capture counts).
pub(crate) fn bg_pose(arr: Vec<u8>, w: usize, h: usize, org: (i32, i32), palette: u8) -> Pose {
    let key = digest(&arr, [h as u8, w as u8]);
    Pose {
        key,
        fighter: true,
        field: false,
        cut: false,
        sprites: 0,
        first: 0,
        priority: 0,
        palette,
        w,
        h,
        arr,
        rgba: Vec::new(),
        org,
        shadow: false,
        excluded: 0,
        recolour: None,
        bg: true,
    }
}

/// The fewest opaque pixels of a shadow.
pub(super) const SHADOW_MIN: usize = 16;

/// The luminance (0 to 255) under which a flat colour is a shadow's.
pub(super) const SHADOW_LUMA: u32 = 110;

/// Whether the opaque pixels of `rgba` are all one dark colour: what the game's ground shadow is
/// (a flat grey ellipse under each fighter). The class 2 capture redrew these as a bright flash,
/// so they are drawn from the game's own shape and colour instead ([`super::draw::draw_shadows`]).
fn flat_dark(rgba: &[u8]) -> bool {
    let mut first: Option<[u8; 3]> = None;
    let mut count = 0;
    for pixel in rgba.as_chunks::<4>().0.iter().filter(|pixel| pixel[3] != 0) {
        let color = [pixel[0], pixel[1], pixel[2]];
        if *first.get_or_insert(color) != color {
            return false;
        }
        count += 1;
    }
    let Some([r, g, b]) = first else { return false };
    let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
    count >= SHADOW_MIN && luma < SHADOW_LUMA
}

/// Whether the opaque pixels of a `w` x `h` array are an ellipse laid on the ground: wide and low
/// (at least one and a half times as wide as high), filling at least 45 percent of the box (an
/// ellipse fills 78), one run of pixels in every row, the runs widening and then narrowing, and
/// about as much to each side of the middle. A character's dark flat parts (the hair of the field's
/// Goku) are not this shape.
pub(super) fn ellipse(arr: &[u8], w: usize, h: usize) -> bool {
    if 2 * w < 3 * h || h == 0 {
        return false;
    }
    let opaque = arr.iter().filter(|&&c| c != 0).count();
    if 100 * opaque < 45 * w * h {
        return false;
    }
    let (mut before, mut narrowing) = (0, false);
    let (mut first_run, mut widest, mut last_run) = (0, 0, 0);
    for (n, row) in arr.chunks(w).enumerate() {
        let Some(first) = row.iter().position(|&c| c != 0) else { return false };
        let last = row.iter().rposition(|&c| c != 0).unwrap_or(first);
        if row[first..=last].contains(&0) || (first + last + 1).abs_diff(w) > w / 4 + 2 {
            return false;
        }
        let run = last + 1 - first;
        if run < before {
            narrowing = true;
        } else if run > before && narrowing {
            return false;
        }
        before = run;
        widest = widest.max(run);
        if n == 0 {
            first_run = run;
        }
        last_run = run;
    }
    first_run < widest && last_run < widest
}

/// Poses of the sprites of the palettes 3 to 7 read as a fighter's (keyed over their palette indexes,
/// clusters of pieces at most 12 pixels apart): the game draws some enemies of a formation with the
/// palettes after the two fighter ones, and those are the same sprites as the units that use palette
/// 1 or 2. They are only candidates: the renderer keeps those whose pixels (or their mirror image)
/// the art index has art for.
pub(crate) fn fighter_candidates(sprites: &[Sprite]) -> Vec<Pose> {
    let none = |_: &Sprite, _: usize, _: u8| [0_u8; 3];
    let mut out = Vec::new();
    for pal in 3_u8..8 {
        let group: Vec<&Sprite> = sprites.iter().filter(|s| s.pal == pal && s.y < 170).collect();
        if group.is_empty() {
            continue;
        }
        for cluster in components(&group, 12) {
            let part: Vec<&Sprite> = cluster.iter().map(|&i| group[i]).collect();
            if let Some(pose) = assemble(&part, true, &none)
                && pose.arr.iter().filter(|&&c| c != 0).count() >= 40
            {
                out.push(pose);
            }
        }
    }
    out
}

/// The poses of the frame that the art index might hold, in the order to try them: for each
/// fighter palette its clusters of pieces, then its whole group; then the effects.
#[cfg(test)]
pub(crate) fn poses(log: &FrameLog, sprites: &[Sprite]) -> Vec<Pose> {
    poses_in(log, sprites, &View::window())
}

/// [`poses`] for the sprites of `view`: a field group is cut where it meets the view's edge.
pub(crate) fn poses_in(log: &FrameLog, sprites: &[Sprite], view: &View) -> Vec<Pose> {
    let mut out = Vec::new();
    for pal in [1_u8, 2] {
        let group: Vec<&Sprite> = sprites.iter().filter(|s| s.pal == pal && s.y < 170).collect();
        if group.is_empty() {
            continue;
        }
        let none = |_: &Sprite, _: usize, _: u8| [0_u8; 3];
        let clusters = components(&group, 12);
        let mut found = Vec::new();
        for cluster in &clusters {
            let part: Vec<&Sprite> = cluster.iter().map(|&i| group[i]).collect();
            if let Some(pose) = assemble(&part, true, &none)
                && pose.arr.iter().filter(|&&c| c != 0).count() >= 40
            {
                found.push(pose);
            }
        }
        out.extend(found);
        // The whole palette group, with the pieces below line 170 too (as the first capture took it)
        let whole: Vec<&Sprite> = sprites.iter().filter(|s| s.pal == pal).collect();
        if let Some(pose) = assemble(&whole, true, &none)
            && (clusters.len() > 1 || whole.len() != group.len())
        {
            out.push(pose);
        }
    }
    let effects: Vec<&Sprite> =
        sprites.iter().filter(|s| !matches!(s.pal, 1 | 2) && s.y < 160).collect();
    if !effects.is_empty() {
        let palettes = Palettes::of(log);
        let colors = |s: &Sprite, row: usize, color: u8| {
            palettes.color(log, s.y + 1 + row as i32 + 1, s.pal, color)
        };
        for cluster in components(&effects, 4) {
            let group: Vec<&Sprite> = cluster.iter().map(|&i| effects[i]).collect();
            let Some(mut pose) = assemble(&group, false, &colors) else { continue };
            // A shadow that touches a flash (a hit's burst on the ground) is a pose of its own,
            // and the cluster's art is not drawn over it. It is a pose only where it is a ground
            // shadow: an ellipse below the rest of the cluster, never a dark flat part of a
            // character (hair)
            if !pose.shadow
                && let Some(shadow) = shadow_candidates(&group, &colors)
                    .into_iter()
                    .filter_map(|piece| assemble(&piece, false, &colors))
                    .find(|shadow| shadow.shadow && below(shadow, &group))
            {
                pose.excluded = shadow.sprites;
                out.push(shadow);
            }
            out.push(pose);
        }
    }
    // The field's sprite groups (class 3): keyed over their palette indexes like a fighter's,
    // and never taken for a fighter
    let none = |_: &Sprite, _: usize, _: u8| [0_u8; 3];
    for group in field_groups(sprites) {
        if let Some(mut pose) = assemble(&group, true, &none)
            && pose.arr.iter().filter(|&&c| c != 0).count() >= FIELD_MIN
        {
            pose.fighter = false;
            pose.field = true;
            pose.cut = group.iter().any(|s| at_edge(s, log.height, view));
            out.push(pose);
        }
    }
    out
}

/// The fewest opaque pixels of a field group (the capture left smaller ones out).
const FIELD_MIN: usize = 40;

/// Whether the box of `sprite` crosses or is within the gap of a field group's pieces from the
/// edge of the view (`height` lines), where the piece next to it may be off the screen.
fn at_edge(sprite: &Sprite, height: u16, view: &View) -> bool {
    sprite.x < view.left + FIELD_GAP
        || sprite.x + sprite.w as i32 > view.right - FIELD_GAP
        || sprite.y + 1 < FIELD_GAP
        || sprite.y + 1 + sprite.h as i32 > i32::from(height) - FIELD_GAP
}

/// How far apart the boxes of two sprites of a field group may be.
const FIELD_GAP: i32 = 2;

/// The lines [`sprites`] keeps: one that starts at line 224 or below is not a sprite of the
/// capture's, though a frame of 239 lines shows it.
const KEPT_LINES: i32 = 224;

impl Pose {
    /// Whether the group runs into the top or the bottom of the picture, where the capture's sprites
    /// end ([`KEPT_LINES`]): the pieces beyond are not in it, whatever the view is made of.
    pub(crate) fn at_vertical_edge(&self) -> bool {
        self.org.1 + ROW_OF_CAPTURE < FIELD_GAP
            || self.org.1 + self.h as i32 + FIELD_GAP > KEPT_LINES
    }
}

/// The sprite groups of the field: the frame's visible sprites in OAM order are split into runs of
/// consecutive entries of one palette, and each run into connected sets (a sprite joins a set when
/// its box is within 2 pixels of one of the set's).
fn field_groups(sprites: &[Sprite]) -> Vec<Vec<&Sprite>> {
    const GAP: i32 = FIELD_GAP;
    let mut out = Vec::new();
    let mut start = 0;
    while start < sprites.len() {
        let mut end = start + 1;
        while end < sprites.len() && sprites[end].pal == sprites[start].pal {
            end += 1;
        }
        let run: Vec<&Sprite> = sprites[start..end].iter().collect();
        for cluster in components(&run, GAP) {
            out.push(cluster.iter().map(|&i| run[i]).collect());
        }
        start = end;
    }
    out
}

/// Whether `shadow` lies below the rest of the pieces of `group`: its middle is in the lower half
/// of the rest's box. A ground shadow is at the character's feet, its hair is not.
fn below(shadow: &Pose, group: &[&Sprite]) -> bool {
    let mut rest = group.iter().filter(|s| shadow.sprites >> s.idx & 1 == 0);
    let Some(first) = rest.next() else { return true };
    let (mut top, mut bottom) = (first.y + 1, first.y + 1 + first.h as i32);
    for s in rest {
        top = top.min(s.y + 1);
        bottom = bottom.max(s.y + 1 + s.h as i32);
    }
    2 * (shadow.org.1 + shadow.h as i32 / 2) >= top + bottom
}

/// The sets of pieces of `group` that may be a ground shadow: the pieces whose opaque pixels are
/// all of one dark colour, a set for each such colour, the one that most pixels have first.
fn shadow_candidates<'a>(
    group: &[&'a Sprite],
    colors: &dyn Fn(&Sprite, usize, u8) -> [u8; 3],
) -> Vec<Vec<&'a Sprite>> {
    let mut flat: Vec<(&Sprite, [u8; 3], usize)> = Vec::new();
    'pieces: for &piece in group {
        let mut first: Option<[u8; 3]> = None;
        let mut count = 0;
        for row in 0..piece.h {
            for column in 0..piece.w {
                let color = piece.pix[row * piece.w + column];
                if color == 0 {
                    continue;
                }
                let rgb = colors(piece, row, color);
                if *first.get_or_insert(rgb) != rgb {
                    continue 'pieces;
                }
                count += 1;
            }
        }
        if let Some(rgb) = first.filter(|&[r, g, b]| {
            (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000 < SHADOW_LUMA
        }) {
            flat.push((piece, rgb, count));
        }
    }
    let mut colours: Vec<[u8; 3]> = flat.iter().map(|(_, rgb, _)| *rgb).collect();
    colours.sort_unstable();
    colours.dedup();
    let share = |rgb: [u8; 3]| -> usize {
        flat.iter().filter(|(_, other, _)| *other == rgb).map(|(_, _, count)| count).sum()
    };
    colours.sort_by_key(|&rgb| std::cmp::Reverse(share(rgb)));
    colours
        .into_iter()
        .map(|rgb| {
            flat.iter().filter(|(_, other, _)| *other == rgb).map(|(piece, _, _)| *piece).collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(idx: u8, x: i32, y: i32, pal: u8, pix: Vec<u8>) -> Sprite {
        Sprite { idx, x, y, w: 8, h: 8, pal, priority: 2, pix }
    }

    fn filled(color: u8) -> Vec<u8> {
        vec![color; 64]
    }

    fn none(_: &Sprite, _: usize, _: u8) -> [u8; 3] {
        [0; 3]
    }

    #[test]
    fn a_pose_is_trimmed_and_keyed_over_its_indexes_then_height_and_width() {
        let mut pix = vec![0_u8; 64];
        pix[8 + 2] = 5; // row 1, column 2
        pix[3 * 8 + 4] = 6; // row 3, column 4
        let s = sprite(3, 10, 20, 1, pix);
        let pose = assemble(&[&s], true, &none).unwrap();
        assert_eq!((pose.w, pose.h), (3, 3));
        assert_eq!(pose.arr, [5, 0, 0, 0, 0, 0, 0, 0, 6]);
        // The origin is the capture's: one row below the sprite's y, plus the trimmed rows
        assert_eq!(pose.org, (12, 20 + 1 + 1));
        let mut hash = Sha1::new();
        hash.update(&pose.arr);
        hash.update(&[3, 3]);
        assert_eq!(pose.key, u64::from_be_bytes(hash.digest().bytes()[..8].try_into().unwrap()));
    }

    #[test]
    fn a_lower_oam_index_is_on_top_where_pieces_overlap() {
        let a = sprite(1, 0, 0, 1, filled(7));
        let b = sprite(2, 4, 0, 1, filled(9));
        let pose = assemble(&[&b, &a], true, &none).unwrap();
        assert_eq!(pose.w, 12);
        assert_eq!(pose.arr[0], 7);
        assert_eq!(pose.arr[4], 7, "the overlap keeps the lower entry");
        assert_eq!(pose.arr[8], 9);
        assert_eq!((pose.first, pose.sprites), (1, 0b110));
    }

    #[test]
    fn an_effect_is_keyed_over_its_colours_not_its_indexes() {
        let piece = sprite(0, 0, 0, 4, filled(2));
        let red = |_: &Sprite, _: usize, _: u8| [255, 0, 0];
        let blue = |_: &Sprite, _: usize, _: u8| [0, 0, 255];
        let in_red = assemble(&[&piece], false, &red).unwrap();
        let in_blue = assemble(&[&piece], false, &blue).unwrap();
        assert_ne!(in_red.key, in_blue.key);
        assert_eq!(in_red.arr, in_blue.arr);
        let fighter_red = assemble(&[&piece], true, &red).unwrap();
        let fighter_blue = assemble(&[&piece], true, &blue).unwrap();
        assert_eq!(fighter_red.key, fighter_blue.key, "a fighter's key does not read colours");
    }

    /// The left (`right` false) or right half of a 16 x 6 ellipse in an 8 x 8 sprite, rows 1 to 6
    /// (widths 8, 12, 16, 16, 12, 8 pixels).
    fn ellipse_half(right: bool) -> Vec<u8> {
        let mut pix = vec![0_u8; 64];
        for (row, width) in [8_usize, 12, 16, 16, 12, 8].into_iter().enumerate() {
            let (from, to) = ((16 - width) / 2, usize::midpoint(16, width));
            for column in from..to {
                if (column >= 8) == right {
                    pix[(row + 1) * 8 + column % 8] = 2;
                }
            }
        }
        pix
    }

    #[test]
    fn an_effect_of_one_dark_flat_ellipse_is_a_shadow_and_nothing_else_is() {
        let dark = |_: &Sprite, _: usize, _: u8| [49, 49, 49];
        let bright = |_: &Sprite, _: usize, _: u8| [250, 250, 250];
        let (left, right) =
            (sprite(0, 0, 0, 4, ellipse_half(false)), sprite(1, 8, 0, 4, ellipse_half(true)));
        assert!(assemble(&[&left, &right], false, &dark).unwrap().shadow);
        assert!(!assemble(&[&left, &right], false, &bright).unwrap().shadow, "too bright");
        assert!(
            !assemble(&[&left, &right], true, &dark).unwrap().shadow,
            "a fighter is not a shadow"
        );
        let two =
            |_: &Sprite, _: usize, color: u8| if color == 2 { [49, 49, 49] } else { [60, 60, 60] };
        let mut pix = ellipse_half(false);
        pix[8 + 3] = 3;
        let mixed = sprite(2, 0, 0, 4, pix);
        assert!(!assemble(&[&mixed, &right], false, &two).unwrap().shadow, "two colours");
        let small = sprite(3, 0, 0, 4, {
            let mut pix = vec![0_u8; 64];
            pix[..8].fill(2);
            pix
        });
        assert!(!assemble(&[&small], false, &dark).unwrap().shadow, "too few pixels");
        // A dark flat block is not the shape of a shadow, nor is a tall one, nor a ring, nor two
        // pieces with a gap (hair)
        let block = sprite(4, 0, 0, 4, filled(2));
        assert!(!assemble(&[&block], false, &dark).unwrap().shadow, "a square");
        let mut tall = vec![0_u8; 64];
        for row in 0..8 {
            tall[row * 8 + 3..row * 8 + 5].fill(2);
        }
        assert!(!assemble(&[&sprite(5, 0, 0, 4, tall)], false, &dark).unwrap().shadow, "tall");
        let apart = sprite(6, 16, 0, 4, ellipse_half(true));
        assert!(
            !assemble(&[&left, &apart], false, &dark).unwrap().shadow,
            "two ellipses far apart"
        );
        let mut ring = ellipse_half(false);
        ring[3 * 8 + 6] = 0;
        ring[3 * 8 + 7] = 0;
        ring[4 * 8 + 6] = 0;
        assert!(
            !assemble(&[&sprite(7, 0, 0, 4, ring), &right], false, &dark).unwrap().shadow,
            "a hole"
        );
    }

    #[test]
    fn a_shadow_touching_a_flash_is_a_pose_of_its_own_and_is_left_out_of_the_flash() {
        let colors = |piece: &Sprite, _: usize, _: u8| {
            if piece.idx < 2 { [49, 49, 49] } else { [250, 240, 200] }
        };
        // Two shadow pieces below a bright one, within four pixels of each other
        let a = sprite(0, 0, 8, 4, ellipse_half(false));
        let b = sprite(1, 8, 8, 4, ellipse_half(true));
        let flash = sprite(2, 4, 0, 4, filled(2));
        let group = [&a, &b, &flash];
        let candidates = shadow_candidates(&group, &colors);
        assert_eq!(candidates.len(), 1);
        let shadow = assemble(&candidates[0], false, &colors).unwrap();
        assert!(shadow.shadow && below(&shadow, &group));
        assert_eq!(shadow.sprites, 0b011);
        let whole = assemble(&group, false, &colors).unwrap();
        assert!(!whole.shadow);
        assert_eq!(whole.sprites, 0b111);
        // A group of only bright pieces has none
        assert!(shadow_candidates(&[&flash], &colors).is_empty());
        // The same dark pieces at the top of the group are not at its feet
        let top = [sprite(0, 0, 0, 4, ellipse_half(false)), sprite(1, 8, 0, 4, ellipse_half(true))];
        let low = sprite(2, 4, 8, 4, filled(2));
        let group = [&top[0], &top[1], &low];
        let candidates = shadow_candidates(&group, &colors);
        let shadow = assemble(&candidates[0], false, &colors).unwrap();
        assert!(shadow.shadow && !below(&shadow, &group), "above the rest: hair, not a shadow");
    }

    #[test]
    fn dark_flat_pieces_of_several_colours_are_tried_commonest_first() {
        let colors = |piece: &Sprite, _: usize, _: u8| {
            if piece.idx == 0 { [0, 0, 0] } else { [40, 40, 40] }
        };
        let hair = sprite(0, 0, 0, 4, filled(2));
        let a = sprite(1, 0, 20, 4, ellipse_half(false));
        let b = sprite(2, 8, 20, 4, ellipse_half(true));
        let group = [&hair, &a, &b];
        let candidates = shadow_candidates(&group, &colors);
        assert_eq!(candidates.len(), 2);
        // The hair has more pixels than either half, but the shadow's pieces (together) more
        assert_eq!(candidates[0].len(), 2);
        assert_eq!(candidates[1].len(), 1);
    }

    #[test]
    fn field_groups_are_runs_of_one_palette_split_into_connected_sets() {
        let sprites = [
            sprite(0, 0, 0, 4, filled(1)),
            sprite(1, 9, 0, 4, filled(1)),
            sprite(2, 100, 0, 4, filled(1)),
            sprite(3, 0, 0, 5, filled(1)),
            sprite(4, 8, 0, 4, filled(1)),
        ];
        let groups = field_groups(&sprites);
        let idx: Vec<Vec<u8>> = groups.iter().map(|g| g.iter().map(|s| s.idx).collect()).collect();
        // Palette 4 for entries 0 to 2 (the first two touch within the gap, the third is apart),
        // then entry 3 of palette 5 on its own, then entry 4: a new run
        assert_eq!(idx, vec![vec![0, 1], vec![2], vec![3], vec![4]]);
    }

    #[test]
    fn a_shadows_ellipse_is_wide_filled_and_one_run_a_row() {
        let rows = |widths: &[usize], w: usize| -> Vec<u8> {
            widths
                .iter()
                .flat_map(|&n| {
                    (0..w).map(move |x| u8::from(x >= (w - n) / 2 && x < usize::midpoint(w, n)))
                })
                .collect()
        };
        assert!(ellipse(&rows(&[6, 10, 10, 6], 10), 10, 4));
        assert!(!ellipse(&rows(&[8, 8, 8, 8], 8), 8, 4), "a bar has no rounded ends");
        assert!(!ellipse(&rows(&[8, 8, 6, 4], 8), 8, 4), "round on one side only");
        assert!(!ellipse(&rows(&[4, 8, 4, 8, 4], 8), 8, 5), "narrowing and widening again");
        assert!(!ellipse(&rows(&[2, 2, 2, 2, 2, 2], 6), 6, 6), "tall");
    }

    #[test]
    fn pieces_within_the_gap_make_one_cluster() {
        let a = sprite(0, 0, 0, 1, filled(1));
        let b = sprite(1, 10, 0, 1, filled(1));
        let c = sprite(2, 100, 0, 1, filled(1));
        let all = [&a, &b, &c];
        assert_eq!(components(&all, 4), vec![vec![0, 1], vec![2]]);
        assert_eq!(components(&all, 1), vec![vec![0], vec![1], vec![2]]);
    }
}
