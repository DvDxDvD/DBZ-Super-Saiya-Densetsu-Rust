//! Save-state file layout, format 3 (all integers little-endian):
//! magic "SSDSTATE" | format u32 | crc32 u32 | `rom_crc32` u32 | frames u64 | `payload_len` u32 |
//! `core_version_len` u32 | `core_version` bytes | payload (bincode, limited by `STATE_CONFIG`).
//! `crc32` covers every byte after the `crc32` field up to the end of the file, so a flipped bit
//! anywhere in the header or the payload is caught. bincode has no integrity check and allocates
//! from length prefixes, so the CRC is verified (after magic and format) before anything else is
//! parsed, and decoding uses a size limit.

use bincode::config::{Configuration, Limit, LittleEndian, Varint};

pub(crate) type StateConfig = Configuration<LittleEndian, Varint, Limit<{ 64 * 1024 * 1024 }>>;
pub(crate) const STATE_CONFIG: StateConfig =
    bincode::config::standard().with_limit::<{ 64 * 1024 * 1024 }>();

const MAGIC: &[u8; 8] = b"SSDSTATE";
const FORMAT_VERSION: u32 = 3;

pub(crate) struct Header {
    pub frames: u64,
}

pub(crate) fn wrap(core_version: &str, rom_crc32: u32, frames: u64, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(payload.len() + 64);
    body.extend_from_slice(&rom_crc32.to_le_bytes());
    body.extend_from_slice(&frames.to_le_bytes());
    body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    body.extend_from_slice(&(core_version.len() as u32).to_le_bytes());
    body.extend_from_slice(core_version.as_bytes());
    body.extend_from_slice(payload);

    let mut out = Vec::with_capacity(body.len() + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&ssd_rom::crc32(&body).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("truncated save state".to_owned());
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8 bytes")))
    }
}

pub(crate) fn unwrap<'a>(
    bytes: &'a [u8],
    core_version: &str,
    rom_crc32: u32,
) -> Result<(Header, &'a [u8]), String> {
    let mut r = Reader(bytes);
    if r.take(MAGIC.len()).ok() != Some(MAGIC.as_slice()) {
        return Err("not a save state".to_owned());
    }
    let format = r.u32()?;
    if format != FORMAT_VERSION {
        return Err(format!("save state format {format}, expected {FORMAT_VERSION}"));
    }
    let crc = r.u32()?;
    if ssd_rom::crc32(r.0) != crc {
        return Err("save state is corrupted (checksum mismatch)".to_owned());
    }
    let state_rom_crc = r.u32()?;
    if state_rom_crc != rom_crc32 {
        return Err(format!(
            "save state is for ROM {state_rom_crc:08X}, current ROM is {rom_crc32:08X}"
        ));
    }
    let frames = r.u64()?;
    let payload_len = r.u32()? as usize;
    let ver_len = r.u32()? as usize;
    if r.take(ver_len)? != core_version.as_bytes() {
        return Err("save state was made by a different emulator core version".to_owned());
    }
    let payload = r.take(payload_len)?;
    if !r.0.is_empty() {
        return Err("trailing bytes in save state".to_owned());
    }
    Ok((Header { frames }, payload))
}
