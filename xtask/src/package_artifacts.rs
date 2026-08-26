use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};
use zip::ZipArchive;

use crate::release_signing::{embedded_public_key, verify_signature_hex};
use crate::sha256::{sha256_file, sha256_reader};

const DEFAULT_MAX_RELEASE_EXE_BYTES: u64 = 15 * 1024 * 1024;
const DEFAULT_MAX_SETUP_BYTES: u64 = 25 * 1024 * 1024;
const DEFAULT_MAX_STATIC_RESOURCE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SIGNATURE_FILE_BYTES: usize = 1024;
const REQUIRED_PAYLOAD: [&str; 8] = [
    "askbridge.exe",
    "WebView2Loader.dll",
    "README.md",
    "PRIVACY.md",
    "TROUBLESHOOTING.md",
    "Install-AskBridge.ps1",
    "Uninstall-AskBridge.ps1",
    "package.json",
];
const SOURCE_PAYLOAD: [(&str, &str); 5] = [
    ("README.md", "README.md"),
    ("docs/PRIVACY.md", "PRIVACY.md"),
    ("docs/TROUBLESHOOTING.md", "TROUBLESHOOTING.md"),
    ("scripts/Install-AskBridge.ps1", "Install-AskBridge.ps1"),
    ("scripts/Uninstall-AskBridge.ps1", "Uninstall-AskBridge.ps1"),
];

#[derive(Debug)]
pub(crate) struct PackageArtifactOptions {
    artifact_root: PathBuf,
    expected_version: String,
    expected_release_exe_path: PathBuf,
    expected_source_root: PathBuf,
    max_release_exe_bytes: u64,
    max_setup_bytes: u64,
    max_static_resource_bytes: u64,
    require_update_signature: bool,
}

impl PackageArtifactOptions {
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut artifact_root = None;
        let mut expected_version = None;
        let mut expected_release_exe_path = None;
        let mut expected_source_root = None;
        let mut max_release_exe_bytes = DEFAULT_MAX_RELEASE_EXE_BYTES;
        let mut max_setup_bytes = DEFAULT_MAX_SETUP_BYTES;
        let mut max_static_resource_bytes = DEFAULT_MAX_STATIC_RESOURCE_BYTES;
        let mut require_update_signature = false;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--artifact-root" => artifact_root = Some(option_value(&flag, &mut args)?.into()),
                "--expected-version" => expected_version = Some(option_value(&flag, &mut args)?),
                "--expected-release-exe-path" => {
                    expected_release_exe_path = Some(option_value(&flag, &mut args)?.into())
                }
                "--expected-source-root" => {
                    expected_source_root = Some(option_value(&flag, &mut args)?.into())
                }
                "--max-release-exe-bytes" => {
                    max_release_exe_bytes =
                        parse_positive_bytes(&flag, &option_value(&flag, &mut args)?)?
                }
                "--max-setup-bytes" => {
                    max_setup_bytes = parse_positive_bytes(&flag, &option_value(&flag, &mut args)?)?
                }
                "--max-static-resource-bytes" => {
                    max_static_resource_bytes =
                        parse_positive_bytes(&flag, &option_value(&flag, &mut args)?)?
                }
                "--require-update-signature" => require_update_signature = true,
                _ => return Err(format!("unknown option '{flag}'")),
            }
        }
        let expected_version = expected_version
            .filter(|version: &String| !version.trim().is_empty())
            .ok_or_else(|| {
                "ExpectedVersion is required for final package artifact validation.".to_owned()
            })?;
        Ok(Self {
            artifact_root: required_option(
                artifact_root,
                "ArtifactRoot is required for final package artifact validation.",
            )?,
            expected_version,
            expected_release_exe_path: required_option(
                expected_release_exe_path,
                "ExpectedReleaseExePath is required for final package artifact validation.",
            )?,
            expected_source_root: required_option(
                expected_source_root,
                "ExpectedSourceRoot is required for final package artifact validation.",
            )?,
            max_release_exe_bytes,
            max_setup_bytes,
            max_static_resource_bytes,
            require_update_signature,
        })
    }
}

pub(crate) fn validate_package_artifacts(options: &PackageArtifactOptions) -> Result<(), String> {
    let artifact_root = required_directory(&options.artifact_root, "ArtifactRoot")?;
    let artifacts = discover_artifacts(
        &artifact_root,
        &options.expected_version,
        options.require_update_signature,
    )?;
    validate_file_header(&artifacts.zip, &[0x50, 0x4B], "Portable ZIP")?;
    validate_portable_executable(&artifacts.setup, "Setup EXE")?;

    let portable_files = validate_portable_payload(&artifacts.portable)?;
    validate_zip(&artifacts.zip, &artifacts.portable, &portable_files)?;
    validate_hash_manifest(&artifact_root, &artifacts)?;
    if let Some(signature) = &artifacts.signature {
        validate_signature_file(signature, &artifacts.hashes)?;
    }
    validate_metadata(&artifacts.portable, &options.expected_version)?;
    validate_identity_and_sizes(options, &artifacts)?;
    Ok(())
}

struct ArtifactSet {
    portable: PathBuf,
    zip: PathBuf,
    setup: PathBuf,
    hashes: PathBuf,
    signature: Option<PathBuf>,
}

fn discover_artifacts(
    root: &Path,
    version: &str,
    require_update_signature: bool,
) -> Result<ArtifactSet, String> {
    let entries = read_entries(root)?;
    let portable = single_matching(&entries, true, "portable directory", |name| {
        name.starts_with("AskBridge-")
    })?;
    let zip = single_matching(&entries, false, "portable ZIP", |name| {
        name.starts_with("AskBridge-") && name.ends_with("-windows-x64.zip")
    })?;
    let setup = single_matching(&entries, false, "Setup EXE", |name| {
        name.starts_with("AskBridge-") && name.ends_with("-Setup.exe")
    })?;
    let hashes = single_matching(&entries, false, "SHA256SUMS file", |name| {
        name.starts_with("AskBridge-") && name.ends_with("-SHA256SUMS.txt")
    })?;
    let signature = optional_matching(&entries, false, "update signature file", |name| {
        name.starts_with("AskBridge-") && name.ends_with("-SHA256SUMS.txt.sig")
    })?;
    if require_update_signature && signature.is_none() {
        return Err(
            "Artifact output is missing the required SHA256SUMS update signature (.sig)."
                .to_owned(),
        );
    }

    let mut allowed: HashSet<OsString> = [&portable, &zip, &setup, &hashes]
        .into_iter()
        .filter_map(|path| path.file_name().map(ToOwned::to_owned))
        .collect();
    if let Some(signature) = &signature {
        allowed.extend(signature.file_name().map(ToOwned::to_owned));
    }
    let unexpected: Vec<String> = entries
        .iter()
        .filter(|entry| !allowed.contains(&entry.file_name()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    if !unexpected.is_empty() {
        return Err(format!(
            "Artifact output contains unexpected top-level items: {}",
            unexpected.join("; ")
        ));
    }

    let expected_name = format!("AskBridge-{version}");
    if !file_name(&portable)?.eq_ignore_ascii_case(&expected_name)
        || !file_name(&zip)?.eq_ignore_ascii_case(&format!("{expected_name}-windows-x64.zip"))
        || !file_name(&setup)?.eq_ignore_ascii_case(&format!("{expected_name}-Setup.exe"))
        || !file_name(&hashes)?.eq_ignore_ascii_case(&format!("{expected_name}-SHA256SUMS.txt"))
    {
        return Err(format!(
            "Artifact names do not match expected version {version}."
        ));
    }
    if let Some(signature) = &signature
        && !file_name(signature)?
            .eq_ignore_ascii_case(&format!("{expected_name}-SHA256SUMS.txt.sig"))
    {
        return Err(format!(
            "Artifact names do not match expected version {version}."
        ));
    }
    Ok(ArtifactSet {
        portable,
        zip,
        setup,
        hashes,
        signature,
    })
}

fn validate_portable_payload(portable: &Path) -> Result<Vec<String>, String> {
    for name in REQUIRED_PAYLOAD {
        if !portable.join(name).is_file() {
            return Err(format!("Portable package is missing {name}."));
        }
    }

    let recursive_files = collect_files(portable)?;
    let unexpected_runtime: Vec<String> = recursive_files
        .iter()
        .filter(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let extension = path
                .extension()
                .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            name.contains("chrome")
                || name.contains("rust")
                || name.contains("cargo")
                || ((extension == "dll" || extension == "msi")
                    && !name.eq_ignore_ascii_case("WebView2Loader.dll"))
        })
        .map(|path| path.display().to_string())
        .collect();
    if !unexpected_runtime.is_empty() {
        return Err(format!(
            "Package unexpectedly bundled external runtime files: {}",
            unexpected_runtime.join("; ")
        ));
    }

    let entries = read_entries(portable)?;
    let mut files = Vec::new();
    let mut directories = Vec::new();
    for entry in entries {
        let file_type = entry
            .file_type()
            .map_err(|error| format!("reading {}: {error}", entry.path().display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_symlink() {
            return Err(format!(
                "Portable package must not contain symbolic links: {name}"
            ));
        }
        if file_type.is_dir() {
            directories.push(name);
        } else if file_type.is_file() {
            files.push(name);
        } else {
            return Err(format!(
                "Portable package contains unsupported item: {name}"
            ));
        }
    }
    files.sort();
    if let Some(unexpected) = files.iter().find(|name| {
        !REQUIRED_PAYLOAD
            .iter()
            .any(|required| name.eq_ignore_ascii_case(required))
    }) {
        return Err(format!(
            "Portable package contains unexpected files: {unexpected}"
        ));
    }
    if !directories.is_empty() {
        return Err(format!(
            "Portable package must be flat; found directories: {}",
            directories.join("; ")
        ));
    }
    Ok(files)
}

fn validate_zip(zip_path: &Path, portable: &Path, portable_files: &[String]) -> Result<(), String> {
    let file =
        File::open(zip_path).map_err(|error| format!("reading {}: {error}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("reading Portable ZIP {}: {error}", zip_path.display()))?;
    let mut zip_entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("reading Portable ZIP entry {index}: {error}"))?;
        let name = entry.name().replace('/', "\\");
        if entry.is_dir()
            || name.contains('\\')
            || entry.enclosed_name().is_none()
            || name == "."
            || name == ".."
        {
            return Err(format!("Portable ZIP contains unsafe entry: {name}"));
        }
        let hash = sha256_reader(&mut entry)
            .map_err(|error| format!("hashing ZIP entry '{name}': {error}"))?;
        zip_entries.push((name, hash));
    }
    zip_entries.sort_by(|left, right| left.0.cmp(&right.0));
    if zip_entries.len() != portable_files.len() {
        return Err("ZIP entry count does not match portable directory file count.".to_owned());
    }
    for ((zip_name, zip_hash), portable_name) in zip_entries.iter().zip(portable_files) {
        if !zip_name.eq_ignore_ascii_case(portable_name) {
            return Err("ZIP entries do not match the portable directory payload.".to_owned());
        }
        let portable_hash = sha256(&portable.join(portable_name))?;
        if !zip_hash.eq_ignore_ascii_case(&portable_hash) {
            return Err(format!(
                "ZIP entry '{portable_name}' hash does not match the portable directory payload."
            ));
        }
    }
    Ok(())
}

fn validate_hash_manifest(root: &Path, artifacts: &ArtifactSet) -> Result<(), String> {
    let bytes = fs::read(&artifacts.hashes)
        .map_err(|error| format!("reading {}: {error}", artifacts.hashes.display()))?;
    if !bytes.is_ascii() {
        return Err("SHA256SUMS must contain ASCII text only.".to_owned());
    }
    let manifest = String::from_utf8(bytes).expect("ASCII is UTF-8");
    let all_files = collect_files(root)?;
    let mut targets = HashMap::new();
    for raw_line in manifest.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let (expected_hash, leaf) = parse_hash_line(line)?;
        let normalized_leaf = leaf.to_ascii_lowercase();
        if targets.contains_key(&normalized_leaf) {
            return Err(format!("Duplicate hash target in SHA256SUMS: {leaf}"));
        }
        let matches: Vec<&PathBuf> = all_files
            .iter()
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(leaf))
            })
            .collect();
        if matches.len() != 1 {
            return Err(format!("Hash target '{leaf}' is missing or ambiguous."));
        }
        let actual_hash = sha256(matches[0])?;
        if actual_hash != expected_hash {
            return Err(format!("Hash verification failed for '{leaf}'."));
        }
        targets.insert(normalized_leaf, leaf.to_owned());
    }
    let expected = [
        file_name(&artifacts.zip)?,
        file_name(&artifacts.setup)?,
        "askbridge.exe".to_owned(),
    ];
    for leaf in &expected {
        if !targets.contains_key(&leaf.to_ascii_lowercase()) {
            return Err(format!("SHA256SUMS does not include {leaf}."));
        }
    }
    for (normalized_leaf, original_leaf) in &targets {
        if !expected
            .iter()
            .any(|leaf| leaf.eq_ignore_ascii_case(normalized_leaf))
        {
            return Err(format!(
                "SHA256SUMS includes unexpected target: {original_leaf}"
            ));
        }
    }
    Ok(())
}

fn validate_metadata(portable: &Path, version: &str) -> Result<(), String> {
    let path = portable.join("package.json");
    let metadata_bytes =
        fs::read(&path).map_err(|error| format!("reading {}: {error}", path.display()))?;
    let metadata_bytes = metadata_bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(&metadata_bytes);
    let metadata: Value = serde_json::from_slice(metadata_bytes)
        .map_err(|error| format!("parsing {}: {error}", path.display()))?;
    let object = metadata
        .as_object()
        .ok_or_else(|| "Package metadata must be a JSON object.".to_owned())?;
    validate_metadata_shape(object)?;
    require_string_property(object, "product")?;
    require_string_property(object, "version")?;
    require_string_property(object, "architecture")?;
    if !metadata_property(object, "product")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("AskBridge"))
        || !metadata_property(object, "architecture")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("windows-x64"))
    {
        return Err("Package metadata does not preserve the expected 1.0 safety flags.".to_owned());
    }
    require_false_property(object, "auto_submit")?;
    require_false_property(object, "chrome_bundled")?;
    let actual_version = metadata_property(object, "version")
        .and_then(Value::as_str)
        .expect("version was validated as string");
    if !actual_version.eq_ignore_ascii_case(version) {
        return Err(format!(
            "Package metadata version is {actual_version}, expected {version}."
        ));
    }
    Ok(())
}

fn validate_identity_and_sizes(
    options: &PackageArtifactOptions,
    artifacts: &ArtifactSet,
) -> Result<(), String> {
    let release_exe = artifacts.portable.join("askbridge.exe");
    require_size_at_most(&release_exe, options.max_release_exe_bytes, "Release EXE")?;
    let expected_release =
        required_file(&options.expected_release_exe_path, "ExpectedReleaseExePath")?;
    let expected_hash = sha256(&expected_release)?;
    let packaged_hash = sha256(&release_exe)?;
    if packaged_hash != expected_hash {
        return Err(format!(
            "Packaged askbridge.exe hash does not match the expected Release EXE. package={packaged_hash} expected={expected_hash}"
        ));
    }
    let source_root = required_directory(&options.expected_source_root, "ExpectedSourceRoot")?;
    for (source_relative, packaged_name) in SOURCE_PAYLOAD {
        let source = required_file(
            &source_root.join(source_relative),
            &format!("source payload {source_relative}"),
        )?;
        let source_hash = sha256(&source)?;
        let packaged_hash = sha256(&artifacts.portable.join(packaged_name))?;
        if packaged_hash != source_hash {
            return Err(format!(
                "Packaged {packaged_name} hash does not match source {source_relative}."
            ));
        }
    }
    require_size_at_most(&artifacts.setup, options.max_setup_bytes, "Setup EXE")?;
    let static_resource_bytes = read_entries(&artifacts.portable)?
        .into_iter()
        .filter(|entry| {
            let name = entry.file_name();
            entry.path().is_file()
                && !REQUIRED_PAYLOAD
                    .iter()
                    .any(|required| name.to_string_lossy().eq_ignore_ascii_case(required))
        })
        .try_fold(0_u64, |total, entry| {
            let length = entry
                .metadata()
                .map_err(|error| format!("reading {}: {error}", entry.path().display()))?
                .len();
            total
                .checked_add(length)
                .ok_or_else(|| "Static resource size overflow.".to_owned())
        })?;
    if static_resource_bytes > options.max_static_resource_bytes {
        return Err(format!(
            "Static resources are {static_resource_bytes} bytes, expected at most {}.",
            options.max_static_resource_bytes
        ));
    }
    Ok(())
}

fn validate_metadata_shape(object: &Map<String, Value>) -> Result<(), String> {
    let expected: HashSet<&str> = [
        "architecture",
        "auto_submit",
        "chrome_bundled",
        "product",
        "version",
    ]
    .into_iter()
    .collect();
    let actual: HashSet<String> = object
        .keys()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let expected_owned: HashSet<String> = expected.iter().map(|name| (*name).to_owned()).collect();
    if object.len() != expected.len() || expected_owned != actual {
        let mut missing: Vec<&str> = expected
            .iter()
            .copied()
            .filter(|name| !actual.contains(*name))
            .collect();
        let mut unexpected: Vec<&str> = object
            .keys()
            .map(String::as_str)
            .filter(|name| !expected.iter().any(|item| item.eq_ignore_ascii_case(name)))
            .collect();
        missing.sort_unstable();
        unexpected.sort_unstable();
        return Err(format!(
            "Package metadata does not match the expected 1.0 field set. missing={} unexpected={}",
            missing.join(","),
            unexpected.join(",")
        ));
    }
    Ok(())
}

fn require_string_property(object: &Map<String, Value>, name: &str) -> Result<(), String> {
    let Some(value) = metadata_property(object, name).and_then(Value::as_str) else {
        return Err(format!(
            "Package metadata property '{name}' must be a JSON string."
        ));
    };
    if value.trim().is_empty() {
        return Err(format!(
            "Package metadata property '{name}' must be non-empty."
        ));
    }
    Ok(())
}

fn require_false_property(object: &Map<String, Value>, name: &str) -> Result<(), String> {
    match metadata_property(object, name) {
        Some(Value::Bool(false)) => Ok(()),
        Some(Value::Bool(true)) => {
            Err(format!("Package metadata property '{name}' must be false."))
        }
        _ => Err(format!(
            "Package metadata property '{name}' must be the JSON boolean false."
        )),
    }
}

fn metadata_property<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    object
        .iter()
        .find(|(property, _)| property.eq_ignore_ascii_case(name))
        .map(|(_, value)| value)
}

fn validate_file_header(path: &Path, prefix: &[u8], label: &str) -> Result<(), String> {
    let mut file =
        File::open(path).map_err(|error| format!("reading {}: {error}", path.display()))?;
    let mut actual = vec![0_u8; prefix.len()];
    file.read_exact(&mut actual)
        .map_err(|_| format!("{label} is too small to be a valid artifact."))?;
    if actual != prefix {
        return Err(format!("{label} does not have the expected file header."));
    }
    Ok(())
}

fn validate_portable_executable(path: &Path, label: &str) -> Result<(), String> {
    let mut file =
        File::open(path).map_err(|error| format!("reading {}: {error}", path.display()))?;
    let length = file
        .metadata()
        .map_err(|error| format!("reading {}: {error}", path.display()))?
        .len();
    if length < 0x40 {
        return Err(format!("{label} is too small to be a valid PE artifact."));
    }
    let mut dos = [0_u8; 2];
    file.read_exact(&mut dos)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    if dos != [0x4D, 0x5A] {
        return Err(format!("{label} does not have the expected PE DOS header."));
    }
    file.seek(SeekFrom::Start(0x3C))
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    let mut offset = [0_u8; 4];
    file.read_exact(&mut offset)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    let offset = i32::from_le_bytes(offset);
    if offset < 0x40 || u64::try_from(offset).map_or(true, |offset| offset > length - 4) {
        return Err(format!("{label} has an invalid PE header offset."));
    }
    file.seek(SeekFrom::Start(offset as u64))
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    let mut signature = [0_u8; 4];
    file.read_exact(&mut signature)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    if signature != [0x50, 0x45, 0x00, 0x00] {
        return Err(format!("{label} does not have the expected PE signature."));
    }
    Ok(())
}

fn parse_hash_line(line: &str) -> Result<(&str, &str), String> {
    if line.len() < 67
        || !line.as_bytes()[..64]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(byte))
        || &line.as_bytes()[64..66] != b"  "
        || line[66..].is_empty()
    {
        return Err(format!("Malformed SHA256SUMS line: {line}"));
    }
    Ok((&line[..64], &line[66..]))
}

fn single_matching(
    entries: &[fs::DirEntry],
    directory: bool,
    label: &str,
    predicate: impl Fn(&str) -> bool,
) -> Result<PathBuf, String> {
    let matches = matching_entries(entries, directory, predicate);
    if matches.len() != 1 {
        return Err(format!(
            "Artifact output must contain exactly one {label}; found {}.",
            matches.len()
        ));
    }
    Ok(matches.into_iter().next().expect("one match"))
}

fn optional_matching(
    entries: &[fs::DirEntry],
    directory: bool,
    label: &str,
    predicate: impl Fn(&str) -> bool,
) -> Result<Option<PathBuf>, String> {
    let matches = matching_entries(entries, directory, predicate);
    if matches.len() > 1 {
        return Err(format!(
            "Artifact output must contain at most one {label}; found {}.",
            matches.len()
        ));
    }
    Ok(matches.into_iter().next())
}

fn matching_entries(
    entries: &[fs::DirEntry],
    directory: bool,
    predicate: impl Fn(&str) -> bool,
) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() == directory && !kind.is_symlink())
                && predicate(&entry.file_name().to_string_lossy())
        })
        .map(fs::DirEntry::path)
        .collect()
}

/// Checks the detached maintainer signature over the exact SHA256SUMS bytes.
fn validate_signature_file(signature: &Path, hashes: &Path) -> Result<(), String> {
    validate_signature_file_with_key(signature, hashes, embedded_public_key())
}

fn validate_signature_file_with_key(
    signature: &Path,
    hashes: &Path,
    public_key_hex: &str,
) -> Result<(), String> {
    let message =
        fs::read(hashes).map_err(|error| format!("reading {}: {error}", hashes.display()))?;
    let signature_bytes =
        fs::read(signature).map_err(|error| format!("reading {}: {error}", signature.display()))?;
    if signature_bytes.len() > MAX_SIGNATURE_FILE_BYTES {
        return Err(format!(
            "Update signature file {} is unexpectedly large.",
            signature.display()
        ));
    }
    let signature_text = std::str::from_utf8(&signature_bytes)
        .map_err(|_| "Update signature must be ASCII hex text.".to_owned())?;
    verify_signature_hex(&message, signature_text, public_key_hex).map_err(|error| {
        format!(
            "Update signature verification failed for {}: {error}",
            signature.display()
        )
    })
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in read_entries(&directory)? {
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            if kind.is_symlink() {
                return Err(format!(
                    "Artifact output must not contain symbolic links: {}",
                    path.display()
                ));
            }
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                files.push(path);
            } else {
                return Err(format!(
                    "Artifact output contains unsupported item: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(files)
}

fn required_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    required_path(path, label, true)
}

fn required_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    required_path(path, label, false)
}

fn required_path(path: &Path, label: &str, directory: bool) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an explicit absolute path."));
    }
    let resolved = fs::canonicalize(path)
        .map_err(|_| format!("{label} does not exist: {}", path.display()))?;
    let metadata = fs::metadata(&resolved)
        .map_err(|_| format!("{label} does not exist: {}", resolved.display()))?;
    if metadata.is_dir() != directory {
        return Err(format!("{label} does not exist: {}", resolved.display()));
    }
    Ok(resolved)
}

fn read_entries(path: &Path) -> Result<Vec<fs::DirEntry>, String> {
    fs::read_dir(path)
        .map_err(|error| format!("reading {}: {error}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("reading {}: {error}", path.display()))
}

fn require_size_at_most(path: &Path, maximum: u64, label: &str) -> Result<(), String> {
    let length = fs::metadata(path)
        .map_err(|error| format!("reading {}: {error}", path.display()))?
        .len();
    if length > maximum {
        return Err(format!(
            "{label} is {length} bytes, expected at most {maximum}."
        ));
    }
    Ok(())
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| format!("path has no file name: {}", path.display()))
}

fn sha256(path: &Path) -> Result<String, String> {
    sha256_file(path).map_err(|error| format!("reading {}: {error}", path.display()))
}

fn parse_positive_bytes(flag: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{flag} must be a positive byte count"))
}

fn option_value(flag: &str, args: &mut impl Iterator<Item = String>) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn required_option<T>(value: Option<T>, message: &str) -> Result<T, String> {
    value.ok_or_else(|| message.to_owned())
}

#[cfg(test)]
mod tests;
