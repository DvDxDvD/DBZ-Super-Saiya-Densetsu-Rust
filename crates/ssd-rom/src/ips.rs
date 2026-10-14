//! IPS patch format: "PATCH", records (offset u24 BE, size u16 BE, data | RLE), "EOF".

use crate::RomError;

/// Applies an IPS patch to `target`, growing it (zero-filled) when a record writes past the end.
///
/// # Errors
///
/// [`RomError::BadPatch`] when the `PATCH` magic is missing or a record, an RLE record or the `EOF`
/// marker is truncated.
pub fn apply_ips(target: &mut Vec<u8>, patch: &[u8]) -> Result<(), RomError> {
    let bad = |msg: &str| RomError::BadPatch(msg.to_owned());
    let body = patch.strip_prefix(b"PATCH").ok_or_else(|| bad("missing PATCH magic"))?;
    let mut i = 0_usize;
    loop {
        let rest = &body[i..];
        if rest.starts_with(b"EOF") {
            return Ok(());
        }
        if rest.len() < 5 {
            return Err(bad("truncated record header"));
        }
        let offset = usize::from(rest[0]) << 16 | usize::from(rest[1]) << 8 | usize::from(rest[2]);
        let size = usize::from(u16::from_be_bytes([rest[3], rest[4]]));
        let (data, consumed): (Vec<u8>, usize) = if size == 0 {
            if rest.len() < 8 {
                return Err(bad("truncated RLE record"));
            }
            let run = usize::from(u16::from_be_bytes([rest[5], rest[6]]));
            (vec![rest[7]; run], 8)
        } else {
            if rest.len() < 5 + size {
                return Err(bad("truncated record data"));
            }
            (rest[5..5 + size].to_vec(), 5 + size)
        };
        if target.len() < offset + data.len() {
            target.resize(offset + data.len(), 0);
        }
        target[offset..offset + data.len()].copy_from_slice(&data);
        i += consumed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(offset: u32, data: &[u8]) -> Vec<u8> {
        let mut r = offset.to_be_bytes()[1..].to_vec();
        r.extend_from_slice(&(data.len() as u16).to_be_bytes());
        r.extend_from_slice(data);
        r
    }

    #[test]
    fn applies_plain_and_rle_records() {
        let mut patch = b"PATCH".to_vec();
        patch.extend(record(2, &[0xAA, 0xBB]));
        patch.extend_from_slice(&[0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x03, 0x7E]); // RLE: 3 × 0x7E at 5
        patch.extend_from_slice(b"EOF");
        let mut target = vec![0_u8; 8];
        apply_ips(&mut target, &patch).unwrap();
        assert_eq!(target, [0, 0, 0xAA, 0xBB, 0, 0x7E, 0x7E, 0x7E]);
    }

    #[test]
    fn grows_target_when_record_is_past_the_end() {
        let mut patch = b"PATCH".to_vec();
        patch.extend(record(4, &[1, 2]));
        patch.extend_from_slice(b"EOF");
        let mut target = vec![9_u8; 2];
        apply_ips(&mut target, &patch).unwrap();
        assert_eq!(target, [9, 9, 0, 0, 1, 2]);
    }

    #[test]
    fn rejects_bad_magic_and_truncation() {
        let mut target = vec![0_u8; 4];
        assert!(matches!(apply_ips(&mut target, b"NOPE"), Err(RomError::BadPatch(_))));
        assert!(matches!(apply_ips(&mut target, b"PATCH\x00\x00"), Err(RomError::BadPatch(_))));
    }
}
