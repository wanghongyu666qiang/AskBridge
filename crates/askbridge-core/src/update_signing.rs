/// Ed25519 public key (uppercase hex) of the maintainer who signs each
/// release's `SHA256SUMS.txt`. Both the updater (`askbridge-win`) and the
/// packaging validator (`xtask`) verify against this key.
///
/// The matching private key is kept offline and provided to CI as the
/// `UPDATE_SIGNING_KEY` secret. Because verification is anchored here, a
/// compromised GitHub account or release cannot ship an accepted update;
/// rotating the key therefore requires shipping a new client version.
pub const RELEASE_SIGNING_PUBLIC_KEY: &str =
    "9ECFF24BC44CEBFE4D91A52264B16F4AF670F4E362A0249F65E09CE3FE6BB08E";

/// Decodes the embedded release public key into raw bytes for signature
/// verification. Returns `None` if the embedded constant is ever malformed.
pub fn release_signing_key() -> Option<[u8; 32]> {
    hex_to_array(RELEASE_SIGNING_PUBLIC_KEY)
}

/// Shared fixed-length hex decoder. Both update-side consumers used to carry
/// byte-identical copies of this parsing; security-relevant decoding lives in
/// exactly one place.
pub fn hex_to_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.trim();
    if text.len() != N * 2 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut decoded = [0_u8; N];
    for (index, byte) in decoded.iter_mut().enumerate() {
        let high = u32::from_str_radix(&text[2 * index..2 * index + 1], 16).ok()?;
        let low = u32::from_str_radix(&text[2 * index + 1..2 * index + 2], 16).ok()?;
        *byte = ((high << 4) | low) as u8;
    }
    Some(decoded)
}

#[cfg(test)]
mod tests {
    use super::{RELEASE_SIGNING_PUBLIC_KEY, hex_to_array, release_signing_key};

    #[test]
    fn embedded_public_key_is_well_formed_hex() {
        assert_eq!(RELEASE_SIGNING_PUBLIC_KEY.len(), 64);
        assert!(
            RELEASE_SIGNING_PUBLIC_KEY
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        // A placeholder of all zeros would make every signature check fail
        // closed, but it must never be committed accidentally.
        assert!(
            RELEASE_SIGNING_PUBLIC_KEY.bytes().any(|byte| byte != b'0'),
            "release signing public key still looks like the all-zero placeholder"
        );
    }

    #[test]
    fn embedded_public_key_decodes_to_thirty_two_bytes() {
        let key = release_signing_key().expect("embedded key decodes");
        assert_eq!(key.len(), 32);
        assert!(key.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn hex_decoder_rejects_wrong_length_and_characters() {
        assert!(hex_to_array::<4>("0011223").is_none());
        assert!(hex_to_array::<4>("001122zz").is_none());
        assert_eq!(hex_to_array::<4>("00112 33"), None);
        assert_eq!(
            hex_to_array::<4>("00112233").expect("valid"),
            [0x00, 0x11, 0x22, 0x33]
        );
    }
}
