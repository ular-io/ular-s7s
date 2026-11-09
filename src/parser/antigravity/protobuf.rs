//! Minimal protobuf reader for Antigravity `step_payload` blobs.

/// Decodes a varint. Returns (value, next offset).
fn read_varint(buf: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let byte = *buf.get(i)?;
        i += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// Returns the first length-delimited (wire type 2) field value as a slice from the message.
pub(super) fn pb_get_bytes(buf: &[u8], field: u64) -> Option<&[u8]> {
    let mut i = 0;
    while i < buf.len() {
        let (tag, ni) = read_varint(buf, i)?;
        i = ni;
        let fno = tag >> 3;
        let wt = tag & 7;
        match wt {
            0 => {
                let (_, ni) = read_varint(buf, i)?;
                i = ni;
            }
            2 => {
                let (len, ni) = read_varint(buf, i)?;
                i = ni;
                let end = i + len as usize;
                let slice = buf.get(i..end)?;
                if fno == field {
                    return Some(slice);
                }
                i = end;
            }
            5 => i += 4,
            1 => i += 8,
            _ => return None,
        }
    }
    None
}

/// Returns all length-delimited (wire type 2) field values as slices from the message (for repeated fields).
pub(super) fn pb_get_bytes_all(buf: &[u8], field: u64) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let Some((tag, ni)) = read_varint(buf, i) else {
            return out;
        };
        i = ni;
        let fno = tag >> 3;
        let wt = tag & 7;
        match wt {
            0 => {
                let Some((_, ni)) = read_varint(buf, i) else {
                    return out;
                };
                i = ni;
            }
            2 => {
                let Some((len, ni)) = read_varint(buf, i) else {
                    return out;
                };
                i = ni;
                let end = i + len as usize;
                let Some(slice) = buf.get(i..end) else {
                    return out;
                };
                if fno == field {
                    out.push(slice);
                }
                i = end;
            }
            5 => i += 4,
            1 => i += 8,
            _ => return out,
        }
    }
    out
}

/// Returns the first varint (wire type 0) field value from the message.
pub(super) fn pb_get_varint(buf: &[u8], field: u64) -> Option<u64> {
    let mut i = 0;
    while i < buf.len() {
        let (tag, ni) = read_varint(buf, i)?;
        i = ni;
        let fno = tag >> 3;
        let wt = tag & 7;
        match wt {
            0 => {
                let (v, ni) = read_varint(buf, i)?;
                i = ni;
                if fno == field {
                    return Some(v);
                }
            }
            2 => {
                let (len, ni) = read_varint(buf, i)?;
                i = ni + len as usize;
            }
            5 => i += 4,
            1 => i += 8,
            _ => return None,
        }
    }
    None
}
