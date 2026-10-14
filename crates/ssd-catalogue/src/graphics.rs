//! The graphics streams of the ROM: the game's LZ decoder (`00:C559`, the stream format of
//! `docs/re/data/graphics.md`) and the bitplane regroup that turns a decoded stream into the
//! SNES tile layout the game DMAs to VRAM (`00:C68C`). Pure functions of the ROM bytes.

/// One decoded LZ stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    /// The decoded bytes: the stream's size, a multiple of `$20` in the game's data.
    pub data: Vec<u8>,
    /// The bytes of the ROM the stream takes, its size word included.
    pub consumed: usize,
}

/// Decodes the LZ stream that starts at `offset` in a headerless `LoROM` image (the file offset,
/// see [`crate::RomView::offset`]). Streams are read in file order, so one may run on into the
/// next bank, as the game's decoder does.
///
/// Returns `None` when the stream is not valid: it runs past the end of the image, it has no
/// size, or a copy refers to distance zero (the game reads its own output there, which no
/// stream does).
#[must_use]
pub fn lz_decode(rom: &[u8], offset: usize) -> Option<Stream> {
    let size = usize::from(u16::from_le_bytes([*rom.get(offset)?, *rom.get(offset + 1)?]));
    if size == 0 {
        return None;
    }
    let mut pos = offset + 2;
    let mut data: Vec<u8> = Vec::with_capacity(size);
    let mut flags = 0_u8;
    let mut items_left = 0_u8;
    while data.len() < size {
        if items_left == 0 {
            flags = *rom.get(pos)?;
            pos += 1;
            items_left = 8;
        }
        items_left -= 1;
        let literal = flags & 1 != 0;
        flags >>= 1;
        if literal {
            data.push(*rom.get(pos)?);
            pos += 1;
        } else {
            let (first, second) = (*rom.get(pos)?, *rom.get(pos + 1)?);
            pos += 2;
            let length = usize::from(first & 0x0F) + 3;
            let distance = usize::from(second) << 4 | usize::from(first >> 4);
            if distance == 0 {
                return None;
            }
            for _ in 0..length {
                // A source before the start of the output reads as zero.
                let byte = data.len().checked_sub(distance).map_or(0, |from| data[from]);
                data.push(byte);
            }
        }
    }
    data.truncate(size);
    Some(Stream { data, consumed: pos - offset })
}

/// The bitplane regroup of `00:C68C`: each 16-byte block holds plane 0 (8 bytes) then plane 1
/// (8 bytes) as the stream stores them; the SNES layout interleaves the two row by row. A
/// 4-bit tile is two such blocks (planes 0 and 1, then planes 2 and 3).
pub fn regroup(data: &mut [u8]) {
    for block in data.as_chunks_mut::<16>().0 {
        let mut out = [0_u8; 16];
        for row in 0..8 {
            out[2 * row] = block[row];
            out[2 * row + 1] = block[8 + row];
        }
        *block = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_a_copy_decode_in_order() {
        // Size 6. Flags 0b0000_0111: three literals, then a copy (bit clear). The copy's first
        // byte $30 gives length 3 (low nibble 0) and distance 3 (high nibble 3, second byte 0)
        let rom = [6, 0, 0b0000_0111, b'a', b'b', b'c', 0x30, 0x00];
        let stream = lz_decode(&rom, 0).unwrap();
        assert_eq!(stream.data, b"abcabc");
        assert_eq!(stream.consumed, 8);
    }

    #[test]
    fn a_copy_reaching_before_the_output_reads_zero() {
        // Size 5. Two literals, then a copy of 3 at distance 4 (first byte $40): its sources
        // are two bytes before the output (zero) and the first literal
        let rom = [5, 0, 0b0000_0011, b'x', b'y', 0x40, 0x00];
        let stream = lz_decode(&rom, 0).unwrap();
        assert_eq!(stream.data, [b'x', b'y', 0, 0, b'x']);
    }

    #[test]
    fn a_stream_is_read_from_its_offset() {
        // Size 1 at offset 2, flags 1, the literal 7; the bytes around it are not read
        let rom = [0xFF, 0xFF, 1, 0, 1, 7, 0xEE];
        assert_eq!(lz_decode(&rom, 2).unwrap(), Stream { data: vec![7], consumed: 4 });
    }

    #[test]
    fn invalid_streams_are_none() {
        assert_eq!(lz_decode(&[0, 0], 0), None, "no size");
        assert_eq!(lz_decode(&[4], 0), None, "a size byte missing");
        // Size 4: a literal, then a copy whose distance is zero
        assert_eq!(lz_decode(&[4, 0, 0b0000_0001, b'a', 0x03, 0x00], 0), None);
        assert_eq!(lz_decode(&[4, 0, 0b0000_0001, b'a'], 0), None, "the image ends first");
    }

    #[test]
    fn regroup_interleaves_the_planes_row_by_row() {
        let mut block: Vec<u8> = (0..16).collect();
        regroup(&mut block);
        assert_eq!(block, [0, 8, 1, 9, 2, 10, 3, 11, 4, 12, 5, 13, 6, 14, 7, 15]);
    }
}
