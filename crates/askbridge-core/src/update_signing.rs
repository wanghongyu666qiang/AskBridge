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

#[cfg(test)]
mod tests {
    use super::RELEASE_SIGNING_PUBLIC_KEY;

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
}
