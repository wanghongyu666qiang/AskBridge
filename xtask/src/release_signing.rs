use std::{env, fs, path::PathBuf};

use askbridge_core::{RELEASE_SIGNING_PUBLIC_KEY, hex_to_array};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use zeroize::Zeroize;

const SIGNING_KEY_ENV: &str = "ASKBRIDGE_UPDATE_SIGNING_KEY";

pub(crate) struct GenUpdateKeyOptions {
    output: PathBuf,
    force: bool,
}

impl GenUpdateKeyOptions {
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut output = None;
        let mut force = false;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--output" => {
                    output = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| format!("{flag} requires a value"))?,
                    ))
                }
                "--force" => force = true,
                _ => return Err(format!("unknown option '{flag}'")),
            }
        }
        Ok(Self {
            output: output.ok_or_else(|| "--output is required for gen-update-key".to_owned())?,
            force,
        })
    }
}

/// Generates a fresh Ed25519 keypair for signing release SHA256SUMS files.
/// The secret key never leaves this machine; CI receives it as a secret.
pub(crate) fn generate_update_key(options: &GenUpdateKeyOptions) -> Result<(), String> {
    if options.output.exists() && !options.force {
        return Err(format!(
            "refusing to overwrite existing key file {} (pass --force to replace)",
            options.output.display()
        ));
    }
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed).map_err(|error| format!("generating key material: {error}"))?;
    let secret = SigningKey::from_bytes(&seed);
    seed.fill(0);
    let document = format!(
        "{{\n  \"secret_key_hex\": \"{}\",\n  \"public_key_hex\": \"{}\"\n}}\n",
        encode_hex_lower(&secret.to_bytes()),
        encode_hex_lower(secret.verifying_key().as_bytes()),
    );
    fs::write(&options.output, document)
        .map_err(|error| format!("writing {}: {error}", options.output.display()))?;
    println!(
        "Generated an Ed25519 release-signing key pair at {}.",
        options.output.display()
    );
    println!(
        "public_key_hex: {} (must match askbridge-core::RELEASE_SIGNING_PUBLIC_KEY)",
        encode_hex_lower(secret.verifying_key().as_bytes())
    );
    println!(
        "Configure the file's secret_key_hex as the GitHub Actions secret referenced by \
         release.yml, keep the file itself offline, and delete it once stored safely."
    );
    Ok(())
}

pub(crate) struct SignShaOptions {
    key_file: Option<PathBuf>,
    input: PathBuf,
    output: PathBuf,
}

impl SignShaOptions {
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut key_file = None;
        let mut input = None;
        let mut output = None;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--key-file" => key_file = Some(PathBuf::from(value)),
                "--input" => input = Some(PathBuf::from(value)),
                "--output" => output = Some(PathBuf::from(value)),
                _ => return Err(format!("unknown option '{flag}'")),
            }
        }
        Ok(Self {
            key_file,
            input: input.ok_or_else(|| "--input is required for sign-sha256sums".to_owned())?,
            output: output.ok_or_else(|| "--output is required for sign-sha256sums".to_owned())?,
        })
    }
}

/// Signs the exact bytes of a SHA256SUMS.txt with the maintainer's offline
/// Ed25519 key and writes the lowercase-hex signature next to it.
pub(crate) fn sign_sha256sums(options: &SignShaOptions) -> Result<(), String> {
    let mut secret_hex = match &options.key_file {
        Some(path) => read_secret_key(path)?,
        None => env::var(SIGNING_KEY_ENV).map_err(|_| {
            format!("signing requires --key-file or the {SIGNING_KEY_ENV} environment variable")
        })?,
    };
    let secret = parse_secret_key(&secret_hex)?;
    // The process exits right after signing, but scrub the hex representation
    // so the plaintext secret does not linger in freed heap memory.
    secret_hex.zeroize();
    let message = fs::read(&options.input)
        .map_err(|error| format!("reading {}: {error}", options.input.display()))?;
    let signature = secret.sign(&message);
    fs::write(
        &options.output,
        format!("{}\n", encode_hex_lower(&signature.to_bytes())),
    )
    .map_err(|error| format!("writing {}: {error}", options.output.display()))?;
    println!(
        "Signed {} into {}.",
        options.input.display(),
        options.output.display()
    );
    Ok(())
}

/// Verifies a hex-encoded detached Ed25519 signature over `message`.
pub(crate) fn verify_signature_hex(
    message: &[u8],
    signature_text: &str,
    public_key_hex: &str,
) -> Result<(), String> {
    let public_key_bytes = hex_to_array::<32>(public_key_hex.trim())
        .ok_or_else(|| "release public key is not valid 64-character hex".to_owned())?;
    let signature_bytes = hex_to_array::<64>(signature_text.trim())
        .ok_or_else(|| "update signature is not valid 128-character hex".to_owned())?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
        .map_err(|error| format!("release public key is invalid: {error}"))?;
    verifying_key
        .verify(message, &Signature::from_bytes(&signature_bytes))
        .map_err(|_| "update signature verification failed".to_owned())
}

pub(crate) fn embedded_public_key() -> &'static str {
    RELEASE_SIGNING_PUBLIC_KEY
}

fn read_secret_key(path: &std::path::Path) -> Result<String, String> {
    let text =
        fs::read_to_string(path).map_err(|error| format!("reading {}: {error}", path.display()))?;
    let trimmed = text.trim();
    if !trimmed.starts_with('{') {
        return Ok(trimmed.to_owned());
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|error| format!("parsing {}: {error}", path.display()))?;
    parsed
        .get("secret_key_hex")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{} is missing secret_key_hex", path.display()))
}

fn parse_secret_key(hex_text: &str) -> Result<SigningKey, String> {
    let mut seed = hex_to_array::<32>(hex_text.trim())
        .ok_or_else(|| "the signing key must be 64 hex characters".to_owned())?;
    let key = SigningKey::from_bytes(&seed);
    seed.zeroize();
    Ok(key)
}

fn encode_hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MESSAGE: &[u8] = b"CAFE  AskBridge-1.2.3-Setup.exe\n";

    #[test]
    fn round_trip_signature_verifies_and_tampering_fails() {
        let secret = SigningKey::from_bytes(&[9_u8; 32]);
        let signature_hex = encode_hex_lower(&secret.sign(TEST_MESSAGE).to_bytes());
        let public_hex = encode_hex_lower(secret.verifying_key().as_bytes());
        assert!(verify_signature_hex(TEST_MESSAGE, &signature_hex, &public_hex).is_ok());
        assert!(verify_signature_hex(b"other", &signature_hex, &public_hex).is_err());
        assert!(verify_signature_hex(TEST_MESSAGE, "zz", &public_hex).is_err());
    }

    #[test]
    fn generated_key_file_round_trips_through_signing() {
        let root = tempfile::tempdir().expect("root");
        let key_path = root.path().join("key.json");
        generate_update_key(&GenUpdateKeyOptions {
            output: key_path.clone(),
            force: false,
        })
        .expect("generate");
        let secret_hex = read_secret_key(&key_path).expect("secret");
        let secret = parse_secret_key(&secret_hex).expect("parse");
        let signature_hex = encode_hex_lower(&secret.sign(TEST_MESSAGE).to_bytes());
        let public_hex = encode_hex_lower(secret.verifying_key().as_bytes());
        assert!(verify_signature_hex(TEST_MESSAGE, &signature_hex, &public_hex).is_ok());

        // A second generation refuses to clobber the existing file.
        assert!(
            generate_update_key(&GenUpdateKeyOptions {
                output: key_path,
                force: false,
            })
            .is_err()
        );
    }
}
