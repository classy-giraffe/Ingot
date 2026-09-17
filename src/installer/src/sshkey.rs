//! SSH authorized-key line validation: the `[ssh] authorized_keys`
//! entries the installer writes into each initial user's
//! `~/.ssh/authorized_keys`.
//!
//! A line is `<type> <base64 blob> [comment]`. The blob is validated
//! against the OpenSSH wire format: it starts with a big-endian u32
//! string length equal to the key type, immediately followed by the
//! key type string itself. That invariant accepts every real key and
//! rejects garbage without re-implementing per-type key parsing.

/// Key types v1 accepts (standard + OpenSSH `sk-` hardware keys).
const KEY_TYPES: [&str; 7] = [
    "ssh-ed25519",
    "ssh-rsa",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// Validates one authorized-key line.
pub fn validate(line: &str) -> Result<(), String> {
    let line = line.trim();
    if line.is_empty() {
        return Err("empty key".into());
    }
    // The base64 blob itself never contains spaces: the first space
    // separates the type, the last remaining space (if any) separates
    // the comment.
    let (key_type, rest) = line
        .split_once(' ')
        .ok_or("expected '<type> <base64> [comment]'")?;
    let (b64, _comment) = match rest.rfind(' ') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    if key_type.is_empty() || !KEY_TYPES.contains(&key_type) {
        return Err(format!("unknown or empty key type {key_type:?}"));
    }
    if b64.is_empty() {
        return Err("empty base64 blob".into());
    }
    let Some(blob) = base64_decode(b64) else {
        return Err("key blob is not valid base64".into());
    };
    let tl = key_type.len() as u64;
    if blob.len() < 4 + key_type.len() {
        return Err("key blob is too short".into());
    }
    let prefix = u32::from_be_bytes([blob[0], blob[1], blob[2], blob[3]]) as u64;
    if prefix != tl {
        return Err(format!(
            "key blob does not start with the type length ({prefix}, want {tl})"
        ));
    }
    if &blob[4..4 + key_type.len()] != key_type.as_bytes() {
        return Err("key blob type does not match the declared key type".into());
    }
    Ok(())
}

/// Standard base64 (with padding); None on invalid characters or
/// length.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let b: Vec<u8> = s.bytes().collect();
    if b.is_empty() || b.len() % 4 != 0 {
        return None;
    }
    let mut res = Vec::new();
    for chunk in b.chunks(4) {
        let mut word: u32 = 0;
        let mut data = 0usize;
        let mut pad_seen = false;
        for c in chunk {
            if *c == b'=' {
                pad_seen = true;
                continue;
            }
            if pad_seen {
                return None; // data after padding in a group
            }
            word = (word << 6) | u32::from(val(*c)?);
            data += 1;
        }
        let out = match data {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => return None, // all-padding group
        };
        let word = word << ((4 - data) * 6);
        for j in 0..out {
            res.push((word >> (16 - 8 * j)) as u8);
        }
    }
    Some(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a wire-format blob for a type with a few key bytes.
    fn blob(ty: &str) -> String {
        let mut v = Vec::new();
        v.extend_from_slice(&(ty.len() as u32).to_be_bytes());
        v.extend_from_slice(ty.as_bytes());
        v.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0x42]);
        base64_encode_test(&v)
    }

    /// Minimal standard base64 encoder (test helper).
    fn base64_encode_test(v: &[u8]) -> String {
        const A: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for chunk in v.chunks(3) {
            let b = [
                chunk.first().copied().unwrap_or(0),
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let word = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            s.push(A[((word >> 18) & 0x3F) as usize] as char);
            s.push(A[((word >> 12) & 0x3F) as usize] as char);
            s.push(if chunk.len() > 1 { A[((word >> 6) & 0x3F) as usize] } else { b'=' } as char);
            s.push(if chunk.len() > 2 { A[(word & 0x3F) as usize] } else { b'=' } as char);
        }
        s
    }

    #[test]
    fn accepts_real_shaped_keys() {
        for ty in KEY_TYPES {
            let line = format!("{ty} {} user@host", blob(ty));
            assert!(validate(&line).is_ok(), "expected {line:?} to pass");
            // same key without a comment
            let bare = format!("{ty} {}", blob(ty));
            assert!(validate(&bare).is_ok(), "expected {bare:?} to pass");
        }
    }

    #[test]
    fn rejects_garbage() {
        let bad: Vec<String> = vec![
            "".into(),
            "not-a-key".into(),
            "ssh-ed25519".into(),
            "ssh-ed25519 !!!".into(),
            "rsa AAAA user@host".into(),
            "AAA user@host".into(),
            // type/blob mismatch: declared ed25519, blob is an rsa wire blob
            format!("ssh-ed25519 {}", blob("ssh-rsa")),
        ];
        for bad in bad {
            assert!(validate(bad.as_str()).is_err(), "expected {bad:?} to be rejected");
        }
    }

    #[test]
    fn rejects_malformed_base64() {
        assert!(validate("ssh-ed25519 AAA= user@host").is_err());
        assert!(validate("ssh-ed25519 AAAA=A user@host").is_err());
        assert!(validate("ssh-ed25519 AAA== user@host").is_err());
    }
}
