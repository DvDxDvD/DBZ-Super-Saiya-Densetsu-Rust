//! PNG files for the HD pack and the UI snapshots: 8-bit RGBA or 8-bit indexed images, written
//! with stored (uncompressed) deflate blocks, so no image crate is needed. The files are larger
//! than compressed PNGs; they are written once (the pack, the snapshots) and read by this crate
//! only, so the size is the price of a deterministic writer with no dependency. Screenshots take
//! it the other way: [`encode_rgba_compressed`] deflates them, since their size is what the player
//! pays and their bytes are not part of anything reproducible.

use std::io;
use std::path::Path;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];
/// The largest block of stored deflate data (the length field is 16 bits).
const STORED_BLOCK: usize = 0xFFFF;
/// Bytes of the Adler-32 sums that can be added before they need reducing (zlib's NMAX).
const ADLER_CHUNK: usize = 5552;
const ADLER_MODULUS: u32 = 65_521;

/// Writes `rgba` (`width` x `height` pixels, top row first) to `path`, creating its directory.
///
/// # Errors
///
/// The directory or the file cannot be written.
pub fn write_rgba(path: &Path, width: u32, height: u32, rgba: &[u8]) -> io::Result<()> {
    write_file(path, &encode_rgba(width, height, rgba))
}

/// Writes the palette indexes `indices` (`width` x `height`, top row first) of an image with
/// `palette` to `path`: see [`encode_indexed`].
///
/// # Errors
///
/// The directory or the file cannot be written.
pub fn write_indexed(
    path: &Path,
    width: u32,
    height: u32,
    indices: &[u8],
    palette: &[[u8; 3]],
    transparent: Option<u8>,
) -> io::Result<()> {
    write_file(path, &encode_indexed(width, height, indices, palette, transparent))
}

fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}

/// The PNG file of an RGBA image.
///
/// # Panics
///
/// `width` or `height` is zero, or `rgba` is not `width * height * 4` bytes.
#[must_use]
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    assert!(width > 0 && height > 0, "an image has pixels");
    let row = width as usize * 4;
    assert_eq!(rgba.len(), row * height as usize, "{width} x {height} RGBA pixels");
    // 8 bits per sample, colour type 6 (RGBA), deflate, filter method 0, no interlacing.
    finish(width, height, 6, rgba.chunks_exact(row), &[])
}

/// The PNG file of an RGBA image, deflate-compressed. The same image as [`encode_rgba`], but a
/// fraction of the size; for the screenshots, whose bytes nobody depends on. The pack keeps the
/// stored writer above, so its files stay reproducible.
///
/// # Panics
///
/// `width` or `height` is zero, or `rgba` is not `width * height * 4` bytes.
#[must_use]
pub fn encode_rgba_compressed(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    assert!(width > 0 && height > 0, "an image has pixels");
    let row = width as usize * 4;
    assert_eq!(rgba.len(), row * height as usize, "{width} x {height} RGBA pixels");
    let mut out = Vec::new();
    let mut encoder = ::png::Encoder::new(&mut out, width, height);
    encoder.set_color(::png::ColorType::Rgba);
    encoder.set_depth(::png::BitDepth::Eight);
    encoder.set_compression(::png::Compression::Balanced);
    let mut writer = encoder.write_header().expect("encoding a PNG header in memory");
    writer.write_image_data(rgba).expect("encoding PNG pixels in memory");
    writer.finish().expect("finishing a PNG in memory");
    out
}

/// The PNG file of an image of palette indexes: 8-bit colour type 3 with `palette` (at most 256
/// RGB entries). `transparent`, if given, is the index that is fully transparent (the other
/// entries stay opaque).
///
/// # Panics
///
/// `width` or `height` is zero, `indices` is not `width * height` bytes, the palette is empty
/// or longer than 256, or an index of `indices` is outside the palette.
#[must_use]
pub fn encode_indexed(
    width: u32,
    height: u32,
    indices: &[u8],
    palette: &[[u8; 3]],
    transparent: Option<u8>,
) -> Vec<u8> {
    assert!(width > 0 && height > 0, "an image has pixels");
    let row = width as usize;
    assert_eq!(indices.len(), row * height as usize, "{width} x {height} indexes");
    assert!((1..=256).contains(&palette.len()), "a palette of 1 to 256 entries");
    assert!(indices.iter().all(|&i| usize::from(i) < palette.len()), "an index in the palette");
    let plte: Vec<u8> = palette.iter().flatten().copied().collect();
    let mut extra = vec![(*b"PLTE", plte)];
    if let Some(index) = transparent {
        // tRNS gives the alpha of the palette entries from the first: opaque, then the index.
        let mut alpha = vec![255_u8; usize::from(index)];
        alpha.push(0);
        extra.push((*b"tRNS", alpha));
    }
    finish(width, height, 3, indices.chunks_exact(row), &extra)
}

/// The file of an image whose scanlines are `rows` (filter 0 added here), of colour `colour_type`
/// with 8-bit samples; `extra` chunks follow the header.
fn finish<'a>(
    width: u32,
    height: u32,
    colour_type: u8,
    rows: impl Iterator<Item = &'a [u8]>,
    extra: &[([u8; 4], Vec<u8>)],
) -> Vec<u8> {
    // Each scanline starts with its filter type: 0, none.
    let mut raw = Vec::new();
    for line in rows {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, colour_type, 0, 0, 0]);
    let mut out = SIGNATURE.to_vec();
    chunk(&mut out, *b"IHDR", &header);
    for (kind, data) in extra {
        chunk(&mut out, *kind, data);
    }
    chunk(&mut out, *b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, *b"IEND", &[]);
    out
}

/// Appends one chunk: its length, its type, its data and the CRC-32 of type and data.
fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    let length = u32::try_from(data.len()).expect("a chunk is under 4 GiB");
    out.extend_from_slice(&length.to_be_bytes());
    let start = out.len();
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let crc = ssd_rom::crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// `data` as a zlib stream of stored deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    // Deflate with a 32 KiB window; 0x7801 is a multiple of 31, as zlib requires.
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(STORED_BLOCK).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        // BFINAL in the first bit, BTYPE 00 (stored); the rest of the byte is padding.
        out.push(u8::from(last));
        let length = u16::try_from(block.len()).expect("blocks are at most 0xFFFF bytes");
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// The Adler-32 checksum that ends a zlib stream.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for chunk in data.chunks(ADLER_CHUNK) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= ADLER_MODULUS;
        b %= ADLER_MODULUS;
    }
    (b << 16) | a
}

/// Whether `bytes` start like a WebP file (`RIFF`, a size, `WEBP`).
#[must_use]
pub fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

/// The size of an image file, a PNG or a WebP, from its header: no pixel is decoded.
///
/// # Errors
///
/// The bytes are neither.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if is_webp(bytes) {
        let decoder =
            image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        return Ok(decoder.dimensions());
    }
    let reader =
        ::png::Decoder::new(std::io::Cursor::new(bytes)).read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    Ok((info.width, info.height))
}

/// The size of the image file at `path`, a PNG or a WebP, from its header: only the start of the
/// file is read.
///
/// # Errors
///
/// The file cannot be read or is neither.
pub fn file_dimensions(path: &Path) -> Result<(u32, u32), String> {
    use std::io::{BufReader, Read, Seek};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut start = [0_u8; 16];
    let read = file.read(&mut start).map_err(|e| e.to_string())?;
    file.rewind().map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);
    if is_webp(&start[..read]) {
        let decoder = image_webp::WebPDecoder::new(reader).map_err(|e| e.to_string())?;
        return Ok(decoder.dimensions());
    }
    let reader = ::png::Decoder::new(reader).read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    Ok((info.width, info.height))
}

/// Encodes `rgba` (`width` x `height`) as a lossless WebP into `out`, for tests.
///
/// # Panics
///
/// `rgba` is not `width * height * 4` bytes.
pub fn encode_webp_lossless(out: &mut Vec<u8>, rgba: &[u8], width: u32, height: u32) {
    image_webp::WebPEncoder::new(out)
        .encode(rgba, width, height, image_webp::ColorType::Rgba8)
        .expect("a lossless WebP");
}

/// Decodes a WebP file (lossy or lossless, with or without alpha) to RGBA.
fn read_webp(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder =
        image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    let mut buffer = vec![0; decoder.output_buffer_size().ok_or("the image is too large")?];
    decoder.read_image(&mut buffer).map_err(|e| e.to_string())?;
    let rgba = if decoder.has_alpha() {
        buffer
    } else {
        buffer.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect()
    };
    Ok((width, height, rgba))
}

/// Decodes any 8-bit PNG (grey, grey with alpha, RGB, RGBA or indexed, not interlaced or
/// interlaced), or a WebP file, to RGBA: the size and the pixels, row by row. This is the reader
/// of the pack's art, which is a PNG or a WebP whatever the extension says.
///
/// # Errors
///
/// The bytes are not an image the decoders take.
pub fn read_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if is_webp(bytes) {
        return read_webp(bytes);
    }
    let mut decoder = ::png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(::png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("the image is too large")?];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    if info.bit_depth != ::png::BitDepth::Eight {
        return Err(format!("{:?}-bit samples", info.bit_depth));
    }
    let data = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        ::png::ColorType::Rgba => data.to_vec(),
        ::png::ColorType::Rgb => {
            data.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect()
        }
        ::png::ColorType::GrayscaleAlpha => {
            data.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()
        }
        ::png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        ::png::ColorType::Indexed => return Err("an indexed image was not expanded".to_owned()),
    };
    Ok((info.width, info.height, rgba))
}

/// Reads back a file of [`encode_rgba`]: its size and its pixels, checking every chunk's CRC and
/// the zlib checksum. It reads only what the writer writes (stored blocks, filter 0).
///
/// # Panics
///
/// The bytes are not such a file.
#[must_use]
pub fn decode_rgba(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    assert_eq!(bytes[..8], SIGNATURE, "PNG signature");
    let (mut pos, mut size, mut idat) = (8, None, Vec::new());
    loop {
        let length = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data = &bytes[pos + 8..pos + 8 + length];
        let crc =
            u32::from_be_bytes(bytes[pos + 8 + length..pos + 12 + length].try_into().unwrap());
        assert_eq!(crc, ssd_rom::crc32(&bytes[pos + 4..pos + 8 + length]), "CRC of {kind:?}");
        match kind {
            b"IHDR" => {
                let width = u32::from_be_bytes(data[0..4].try_into().unwrap());
                let height = u32::from_be_bytes(data[4..8].try_into().unwrap());
                assert_eq!(data[8..13], [8, 6, 0, 0, 0], "8-bit RGBA, no interlace");
                size = Some((width, height));
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            other => panic!("unexpected chunk {other:?}"),
        }
        pos += 12 + length;
    }
    let (width, height) = size.expect("IHDR comes first");
    assert_eq!(idat[..2], [0x78, 0x01], "zlib header");
    let mut raw = Vec::new();
    let mut p = 2;
    loop {
        assert_eq!(idat[p] & 0b110, 0, "stored block");
        let last = idat[p] & 1 == 1;
        let length = u16::from_le_bytes([idat[p + 1], idat[p + 2]]);
        let inverse = u16::from_le_bytes([idat[p + 3], idat[p + 4]]);
        assert_eq!(inverse, !length, "LEN and NLEN");
        raw.extend_from_slice(&idat[p + 5..p + 5 + usize::from(length)]);
        p += 5 + usize::from(length);
        if last {
            break;
        }
    }
    let checksum = u32::from_be_bytes(idat[p..p + 4].try_into().unwrap());
    assert_eq!(checksum, adler32(&raw), "zlib Adler-32");
    let row = width as usize * 4;
    let mut pixels = Vec::with_capacity(row * height as usize);
    for line in raw.chunks_exact(row + 1) {
        assert_eq!(line[0], 0, "filter 0");
        pixels.extend_from_slice(&line[1..]);
    }
    (width, height, pixels)
}

/// Adler-32 straight from its definition, reduced only at the end: independent of [`adler32`].
#[cfg(test)]
fn naive_adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u64, 0_u64);
    for &byte in data {
        a += u64::from(byte);
        b += a;
    }
    let modulus = u64::from(ADLER_MODULUS);
    (((b % modulus) << 16) | (a % modulus)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|i| [(i % 251) as u8, (i / 7 % 253) as u8, 0x3A, 0xFF])
            .collect()
    }

    #[test]
    fn a_webp_file_reads_as_rgba_like_a_png() {
        let pixels = gradient(5, 4);
        let mut file = Vec::new();
        image_webp::WebPEncoder::new(&mut file)
            .encode(&pixels, 5, 4, image_webp::ColorType::Rgba8)
            .unwrap();
        assert!(is_webp(&file) && !is_webp(&encode_rgba(5, 4, &pixels)));
        assert_eq!(dimensions(&file).unwrap(), (5, 4));
        assert_eq!(dimensions(&encode_rgba(5, 4, &pixels)).unwrap(), (5, 4));
        // The reader takes either format, and a lossless WebP is the same pixels
        assert_eq!(read_rgba(&file).unwrap(), (5, 4, pixels.clone()));
        assert_eq!(read_rgba(&encode_rgba(5, 4, &pixels)).unwrap(), (5, 4, pixels));
        assert!(read_rgba(b"RIFFxxxxWEBPnot an image").is_err());
        // The header alone gives the size of a file
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.webp"), &file).unwrap();
        write_rgba(&dir.path().join("a.png"), 5, 4, &gradient(5, 4)).unwrap();
        assert_eq!(file_dimensions(&dir.path().join("a.webp")).unwrap(), (5, 4));
        assert_eq!(file_dimensions(&dir.path().join("a.png")).unwrap(), (5, 4));
        assert!(file_dimensions(&dir.path().join("missing.png")).is_err());
    }

    #[test]
    fn a_small_image_reads_back_as_it_was_written() {
        let pixels = gradient(3, 2);
        let (width, height, decoded) = decode_rgba(&encode_rgba(3, 2, &pixels));
        assert_eq!((width, height), (3, 2));
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn an_image_of_several_stored_blocks_reads_back_as_it_was_written() {
        // 300 x 100 pixels: about 120 KB of scanlines, so two stored blocks at least.
        let pixels = gradient(300, 100);
        let (width, height, decoded) = decode_rgba(&encode_rgba(300, 100, &pixels));
        assert_eq!((width, height), (300, 100));
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn the_checksum_matches_the_definition_on_long_inputs() {
        let data: Vec<u8> =
            (0..100_000_u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        assert_eq!(adler32(&data), naive_adler32(&data));
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398, "the published example");
    }

    #[test]
    fn a_file_is_written_with_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("made").join("shot.png");
        write_rgba(&path, 2, 1, &[1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        let (width, height, pixels) = decode_rgba(&std::fs::read(&path).unwrap());
        assert_eq!((width, height), (2, 1));
        assert_eq!(pixels, [1, 2, 3, 255, 4, 5, 6, 255]);
    }
}
