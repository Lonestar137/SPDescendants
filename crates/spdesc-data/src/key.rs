//! The container AES key: a *fact about the player's install*, held in the local-state folder and
//! never committed.
//!
//! The key lives in `<config-root>/keys.json` under the `aes256_hex` field. The config root is
//! `$SPDESC_HOME` when set (this checkout's `shell.nix` points it at `spdesc-config/`); a `dev`
//! build falls back to `<checkout>/spdesc-config/`. A shipped build (`default-features = false`)
//! has no fallback, so the key must be supplied through the environment.

use std::path::PathBuf;

/// The local-state root: `$SPDESC_HOME`, or this checkout's `spdesc-config/` in a `dev` build.
pub fn config_root() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("SPDESC_HOME")
        && !home.is_empty()
    {
        return Some(PathBuf::from(home));
    }
    #[cfg(feature = "dev")]
    {
        use std::path::Path;
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .map(|root| root.join("spdesc-config"))
    }
    #[cfg(not(feature = "dev"))]
    {
        None
    }
}

/// Parse the `aes256_hex` field out of a `keys.json` body without a JSON dependency: the file is
/// ours and the field is a single 64-hex-character string.
pub fn parse_aes256_hex(text: &str) -> Option<[u8; 32]> {
    let after = text.split("\"aes256_hex\"").nth(1)?;
    let raw = after.split(':').nth(1)?;
    let hex = raw
        .trim_start()
        .trim_start_matches('"')
        .trim_start_matches("0x")
        .split('"')
        .next()?
        .trim();
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(key)
}

/// The container AES key for this machine, or `None` when the state folder or the field is
/// missing.
pub fn aes_key() -> Option<[u8; 32]> {
    let text = std::fs::read_to_string(config_root()?.join("keys.json")).ok()?;
    parse_aes256_hex(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_hex_key_among_other_fields() {
        let text = r#"{ "aes256_hex": "0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20", "note": "x" }"#;
        let key = parse_aes256_hex(text).expect("key");
        assert_eq!(key[0], 0x01);
        assert_eq!(key[15], 0x10);
        assert_eq!(key[31], 0x20);
    }

    #[test]
    fn rejects_a_short_or_non_hex_key() {
        assert!(parse_aes256_hex(r#"{ "aes256_hex": "abcd" }"#).is_none());
        assert!(parse_aes256_hex(r#"{ "aes256_hex": "zz02030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20" }"#).is_none());
        assert!(parse_aes256_hex("no key here").is_none());
    }
}
