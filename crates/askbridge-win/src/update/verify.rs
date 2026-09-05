//! Verification helpers for downloaded and cached update artifacts: SHA-256
//! checksum extraction, the offline Ed25519 signature over SHA256SUMS, and
//! the re-verification of cached installers before launch.

use std::{fs, io::Read, path::Path};

use askbridge_core::{AppError, Result, Sha256Stream, hex_to_array};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use super::update_error;

/// Verifies the release maintainer's offline Ed25519 signature over the exact
/// SHA256SUMS bytes. The public key is embedded at compile time, so tampering
/// with the GitHub release cannot produce accepted checksums.
pub(super) fn verify_checksum_signature(
    message: &[u8],
    signature_text: &str,
    public_key_hex: &str,
) -> Result<()> {
    let public_key_bytes =
        hex_to_array(public_key_hex.trim()).ok_or_else(|| update_error("内嵌更新公钥格式无效"))?;
    let signature_bytes =
        hex_to_array(signature_text.trim()).ok_or_else(|| update_error("更新签名格式无效"))?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
        .map_err(|_| update_error("内嵌更新公钥无效"))?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify(message, &signature)
        .map_err(|_| update_error("更新签名校验失败，安装包来源不可信"))
}

pub(super) fn expected_checksum(source: &str, expected_name: &str) -> Result<String> {
    let mut found = None;
    for line in source.lines().filter(|line| !line.trim().is_empty()) {
        let Some((hash, name)) = line.split_once("  ") else {
            return Err(update_error("SHA256SUMS 格式无效"));
        };
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(update_error("SHA256SUMS 包含无效哈希"));
        }
        if name == expected_name && found.replace(hash.to_owned()).is_some() {
            return Err(update_error("SHA256SUMS 包含重复安装包记录"));
        }
    }
    found.ok_or_else(|| update_error("SHA256SUMS 未包含更新安装包"))
}

pub(super) fn validate_downloaded_setup(update_root: &Path, setup_path: &Path) -> Result<()> {
    if !update_root.is_absolute()
        || !setup_path.is_absolute()
        || setup_path.parent() != Some(update_root)
        || !setup_path.is_file()
        || !setup_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| is_safe_file_name(name) && name.ends_with("-Setup.exe"))
    {
        return Err(update_error("更新安装包路径不安全"));
    }
    Ok(())
}

/// Re-hashes a cached setup against the `<name>.sha256` record written when it
/// was published. Anything that modified or replaced the file after download —
/// corruption, another process — is rejected and removed instead of launched.
pub(super) fn verify_cached_hash(setup_path: &Path) -> Result<()> {
    let expected = read_hash_record(setup_path)?;
    let actual = hash_file_streaming(setup_path)?;
    if !actual.eq_ignore_ascii_case(&expected) {
        remove_cached_setup(setup_path);
        return Err(update_error("缓存更新安装包与校验记录不一致，请重新下载"));
    }
    Ok(())
}

pub(super) fn read_hash_record(setup_path: &Path) -> Result<String> {
    let Some(name) = setup_path.file_name().and_then(|name| name.to_str()) else {
        return Err(update_error("更新安装包路径不安全"));
    };
    let hash_record = setup_path.with_file_name(format!("{name}.sha256"));
    let recorded = fs::read_to_string(&hash_record)
        .map_err(|_| update_error("缓存更新安装包缺少校验记录，请重新下载"))?;
    let expected = recorded.trim();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(update_error("缓存更新校验记录无效，请重新下载"));
    }
    Ok(expected.to_owned())
}

pub(super) fn remove_cached_setup(setup_path: &Path) {
    let _ = fs::remove_file(setup_path);
    if let Some(name) = setup_path.file_name().and_then(|name| name.to_str()) {
        let _ = fs::remove_file(setup_path.with_file_name(format!("{name}.sha256")));
    }
}

/// Opens the setup with a handle that shares only read access. While it is
/// held, Windows refuses other opens that request write or delete access, so
/// the file cannot be swapped or modified between verification and launch;
/// if another process already holds a writable handle, this open itself fails
/// and the launch is refused.
pub(super) fn hold_exclusive_read(setup_path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(setup_path)
}

pub(super) fn hash_file_streaming(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .map_err(|source| AppError::io("reading cached update", path, source))?;
    let mut hasher = Sha256Stream::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| AppError::io("reading cached update", path, source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish_hex())
}

pub(super) fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn test_hash(bytes: &[u8]) -> String {
        let mut hasher = Sha256Stream::new();
        hasher.update(bytes);
        hasher.finish_hex()
    }

    fn encode_hex_upper(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02X}")).collect()
    }

    #[test]
    fn cached_setup_hash_verification_accepts_match_and_rejects_tampering() {
        let root = tempdir().expect("root");
        let setup = root.path().join("AskBridge-1.2.3-Setup.exe");
        fs::write(&setup, b"installer bytes").expect("setup");

        // A missing record fails closed.
        assert!(verify_cached_hash(&setup).is_err());

        let record = root.path().join("AskBridge-1.2.3-Setup.exe.sha256");
        fs::write(&record, format!("{}\n", test_hash(b"installer bytes"))).expect("record");
        assert!(verify_cached_hash(&setup).is_ok());

        fs::write(&setup, b"tampered bytes").expect("tamper");
        assert!(verify_cached_hash(&setup).is_err());
        // The tampered pair is removed so a retry forces a fresh download.
        assert!(!setup.exists());
        assert!(!record.exists());
    }

    #[test]
    fn exclusive_read_pin_blocks_writers_until_released() {
        let root = tempdir().expect("root");
        let setup = root.path().join("AskBridge-1.2.3-Setup.exe");
        fs::write(&setup, b"installer bytes").expect("setup");

        let pin = hold_exclusive_read(&setup).expect("pin");
        // Writers (and deleters) are refused while the pin is held, which is
        // what keeps the verified bytes from being swapped before launch.
        let writer = fs::OpenOptions::new().write(true).open(&setup);
        assert!(writer.is_err(), "write access must be refused while pinned");
        drop(pin);
        let writer = fs::OpenOptions::new().write(true).open(&setup);
        assert!(writer.is_ok(), "write access must work again after release");
    }

    #[test]
    fn parses_only_exact_setup_checksum() {
        let source = concat!(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA  AskBridge-1.2.3-windows-x64.zip\n",
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB  AskBridge-1.2.3-Setup.exe\n",
        );
        assert_eq!(
            expected_checksum(source, "AskBridge-1.2.3-Setup.exe").expect("checksum"),
            "B".repeat(64)
        );
        assert!(expected_checksum(source, "AskBridge-9.9.9-Setup.exe").is_err());
    }

    #[test]
    fn rejects_duplicate_setup_checksum() {
        let line = format!("{}  AskBridge-1.2.3-Setup.exe\n", "A".repeat(64));
        assert!(expected_checksum(&(line.clone() + &line), "AskBridge-1.2.3-Setup.exe").is_err());
    }

    #[test]
    fn checksums_require_a_valid_ed25519_signature() {
        use ed25519_dalek::{Signer, SigningKey};

        let secret = SigningKey::from_bytes(&[7_u8; 32]);
        let public_hex = encode_hex_upper(secret.verifying_key().as_bytes());
        let message =
            b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA  AskBridge-1.2.3-Setup.exe\n";
        let signature_hex = encode_hex_upper(&secret.sign(message).to_bytes());
        assert!(verify_checksum_signature(message, &signature_hex, &public_hex).is_ok());

        let tampered = b"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB  AskBridge-1.2.3-Setup.exe\n";
        assert!(verify_checksum_signature(tampered, &signature_hex, &public_hex).is_err());
        assert!(verify_checksum_signature(message, "not-hex", &public_hex).is_err());
        let invalid_key_hex: String = "00".repeat(32);
        assert!(verify_checksum_signature(message, &signature_hex, &invalid_key_hex).is_err());
    }

    #[test]
    fn downloaded_setup_must_be_an_existing_direct_child() {
        let root = tempdir().expect("root");
        let updates = root.path().join("Updates");
        fs::create_dir_all(&updates).expect("updates");
        let setup = updates.join("AskBridge-1.2.3-Setup.exe");
        fs::write(&setup, b"setup").expect("setup");
        let absolute_updates = std::path::absolute(&updates).expect("absolute updates");
        let absolute_setup = absolute_updates.join("AskBridge-1.2.3-Setup.exe");
        assert!(validate_downloaded_setup(&absolute_updates, &absolute_setup).is_ok());
        assert!(
            validate_downloaded_setup(&absolute_updates, &root.path().join("outside.exe")).is_err()
        );
    }
}
