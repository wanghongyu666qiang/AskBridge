use std::{fs, fs::File, io::Write, path::Path};

use serde_json::json;
use tempfile::TempDir;
use zip::{CompressionMethod, ZipWriter, write::FileOptions};

use super::{PackageArtifactOptions, sha256, validate_package_artifacts};

struct Fixture {
    _root: TempDir,
    artifact_root: std::path::PathBuf,
    portable_root: std::path::PathBuf,
    source_root: std::path::PathBuf,
    options: PackageArtifactOptions,
}

impl Fixture {
    fn valid() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let artifact_root = root.path().join("artifacts");
        let portable_root = artifact_root.join("AskBridge-9.9.9");
        let source_root = root.path().join("source");
        fs::create_dir_all(&portable_root).expect("portable root");
        fs::create_dir_all(source_root.join("docs")).expect("source docs");
        fs::create_dir_all(source_root.join("scripts")).expect("source scripts");
        let payload = [
            ("askbridge.exe", b"fixture-exe".as_slice()),
            ("WebView2Loader.dll", b"fixture-loader".as_slice()),
            ("README.md", b"readme".as_slice()),
            ("PRIVACY.md", b"privacy".as_slice()),
            ("TROUBLESHOOTING.md", b"troubleshooting".as_slice()),
            ("Install-AskBridge.ps1", b"install".as_slice()),
            ("Uninstall-AskBridge.ps1", b"uninstall".as_slice()),
        ];
        for (name, bytes) in payload {
            fs::write(portable_root.join(name), bytes).expect("payload");
        }
        fs::write(source_root.join("README.md"), b"readme").expect("source readme");
        fs::write(source_root.join("docs/PRIVACY.md"), b"privacy").expect("source privacy");
        fs::write(
            source_root.join("docs/TROUBLESHOOTING.md"),
            b"troubleshooting",
        )
        .expect("source troubleshooting");
        fs::write(
            source_root.join("scripts/Install-AskBridge.ps1"),
            b"install",
        )
        .expect("source install");
        fs::write(
            source_root.join("scripts/Uninstall-AskBridge.ps1"),
            b"uninstall",
        )
        .expect("source uninstall");
        write_metadata(
            &portable_root,
            json!({
                "product": "AskBridge",
                "version": "9.9.9",
                "architecture": "windows-x64",
                "auto_submit": false,
                "chrome_bundled": false
            }),
        );
        let zip = artifact_root.join("AskBridge-9.9.9-windows-x64.zip");
        write_zip(&portable_root, &zip, None);
        let setup = artifact_root.join("AskBridge-9.9.9-Setup.exe");
        write_minimal_pe(&setup);
        write_manifest(&artifact_root, &portable_root, &zip, &setup, &[]);
        let options = PackageArtifactOptions {
            artifact_root: artifact_root.clone(),
            expected_version: "9.9.9".to_owned(),
            expected_release_exe_path: portable_root.join("askbridge.exe"),
            expected_source_root: source_root.clone(),
            max_release_exe_bytes: super::DEFAULT_MAX_RELEASE_EXE_BYTES,
            max_setup_bytes: super::DEFAULT_MAX_SETUP_BYTES,
            max_static_resource_bytes: super::DEFAULT_MAX_STATIC_RESOURCE_BYTES,
        };
        Self {
            _root: root,
            artifact_root,
            portable_root,
            source_root,
            options,
        }
    }

    fn rebuild_zip_and_manifest(&self) {
        let zip = self.artifact_root.join("AskBridge-9.9.9-windows-x64.zip");
        write_zip(&self.portable_root, &zip, None);
        write_manifest(
            &self.artifact_root,
            &self.portable_root,
            &zip,
            &self.artifact_root.join("AskBridge-9.9.9-Setup.exe"),
            &[],
        );
    }
}

#[test]
fn accepts_complete_matching_artifacts() {
    let fixture = Fixture::valid();
    validate_package_artifacts(&fixture.options).expect("valid artifacts");
}

#[test]
fn parses_and_accepts_complete_cli_options() {
    let fixture = Fixture::valid();
    let options = PackageArtifactOptions::parse([
        "--artifact-root".to_owned(),
        fixture.artifact_root.to_string_lossy().into_owned(),
        "--expected-version".to_owned(),
        "9.9.9".to_owned(),
        "--expected-release-exe-path".to_owned(),
        fixture
            .portable_root
            .join("askbridge.exe")
            .to_string_lossy()
            .into_owned(),
        "--expected-source-root".to_owned(),
        fixture.source_root.to_string_lossy().into_owned(),
    ])
    .expect("CLI options");
    validate_package_artifacts(&options).expect("CLI-selected artifacts");
}

#[test]
fn accepts_powershell_utf8_bom_metadata() {
    let fixture = Fixture::valid();
    let path = fixture.portable_root.join("package.json");
    let metadata = fs::read(&path).expect("metadata");
    let mut with_bom = vec![0xEF, 0xBB, 0xBF];
    with_bom.extend(metadata);
    fs::write(path, with_bom).expect("BOM metadata");
    fixture.rebuild_zip_and_manifest();
    validate_package_artifacts(&fixture.options).expect("PowerShell UTF-8 metadata");
}

#[test]
fn parser_requires_final_evidence_and_absolute_paths() {
    let error = PackageArtifactOptions::parse(Vec::<String>::new()).expect_err("missing");
    assert_eq!(
        error,
        "ExpectedVersion is required for final package artifact validation."
    );
    let fixture = Fixture::valid();
    let mut options = fixture.options;
    options.artifact_root = "relative-artifacts".into();
    assert_eq!(
        validate_package_artifacts(&options).expect_err("relative root"),
        "ArtifactRoot must be an explicit absolute path."
    );
}

#[test]
fn rejects_wrong_version_and_unexpected_top_level_items() {
    let mut fixture = Fixture::valid();
    fixture.options.expected_version = "1.0.0".to_owned();
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("wrong version")
            .starts_with("Artifact names do not match expected version")
    );

    let fixture = Fixture::valid();
    fs::write(fixture.artifact_root.join("stale.sed"), b"stale").expect("stale file");
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("unexpected top level")
            .starts_with("Artifact output contains unexpected top-level items:")
    );
}

#[test]
fn rejects_metadata_shape_types_and_safety_flags() {
    let fixture = Fixture::valid();
    write_metadata(
        &fixture.portable_root,
        json!({
            "product": "AskBridge", "version": "9.9.9", "architecture": "windows-x64",
            "auto_submit": false, "chrome_bundled": false, "legacy_auto_send": true
        }),
    );
    fixture.rebuild_zip_and_manifest();
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("extra metadata")
            .starts_with("Package metadata does not match the expected 1.0 field set.")
    );

    let fixture = Fixture::valid();
    write_metadata(
        &fixture.portable_root,
        json!({
            "product": "AskBridge", "version": "9.9.9", "architecture": "windows-x64",
            "auto_submit": "false", "chrome_bundled": false
        }),
    );
    fixture.rebuild_zip_and_manifest();
    assert_eq!(
        validate_package_artifacts(&fixture.options).expect_err("string flag"),
        "Package metadata property 'auto_submit' must be the JSON boolean false."
    );
}

#[test]
fn rejects_hash_manifest_mismatch_and_extra_target() {
    let fixture = Fixture::valid();
    fs::write(
        fixture.artifact_root.join("AskBridge-9.9.9-Setup.exe"),
        minimal_pe_with_suffix(b"changed"),
    )
    .expect("change setup");
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("hash mismatch")
            .starts_with("Hash verification failed for")
    );

    let fixture = Fixture::valid();
    let hash = sha256(&fixture.portable_root.join("README.md")).expect("readme hash");
    let manifest = fixture.artifact_root.join("AskBridge-9.9.9-SHA256SUMS.txt");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(manifest)
        .expect("manifest");
    writeln!(file, "{hash}  README.md").expect("extra hash");
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("extra target")
            .starts_with("SHA256SUMS includes unexpected target:")
    );
}

#[test]
fn rejects_malformed_duplicate_or_incomplete_hash_manifest() {
    let fixture = Fixture::valid();
    let manifest = fixture.artifact_root.join("AskBridge-9.9.9-SHA256SUMS.txt");
    fs::write(&manifest, b"not-a-hash\r\n").expect("malformed manifest");
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("malformed manifest")
            .starts_with("Malformed SHA256SUMS line:")
    );

    let fixture = Fixture::valid();
    let manifest = fixture.artifact_root.join("AskBridge-9.9.9-SHA256SUMS.txt");
    let first = fs::read_to_string(&manifest)
        .expect("manifest")
        .lines()
        .next()
        .expect("first line")
        .to_owned();
    fs::write(&manifest, format!("{first}\r\n{first}\r\n")).expect("duplicate manifest");
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("duplicate target")
            .starts_with("Duplicate hash target in SHA256SUMS:")
    );

    let fixture = Fixture::valid();
    let manifest = fixture.artifact_root.join("AskBridge-9.9.9-SHA256SUMS.txt");
    let only_zip = fs::read_to_string(&manifest)
        .expect("manifest")
        .lines()
        .next()
        .expect("zip line")
        .to_owned();
    fs::write(&manifest, format!("{only_zip}\r\n")).expect("incomplete manifest");
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("missing target")
            .starts_with("SHA256SUMS does not include")
    );
}

#[test]
fn rejects_zip_structure_content_and_unsafe_paths() {
    let fixture = Fixture::valid();
    write_zip(
        &fixture.portable_root,
        &fixture
            .artifact_root
            .join("AskBridge-9.9.9-windows-x64.zip"),
        Some(("README.md", b"tampered")),
    );
    write_manifest(
        &fixture.artifact_root,
        &fixture.portable_root,
        &fixture
            .artifact_root
            .join("AskBridge-9.9.9-windows-x64.zip"),
        &fixture.artifact_root.join("AskBridge-9.9.9-Setup.exe"),
        &[],
    );
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("zip content")
            .starts_with("ZIP entry 'README.md' hash does not match")
    );

    let fixture = Fixture::valid();
    write_unsafe_zip(
        &fixture
            .artifact_root
            .join("AskBridge-9.9.9-windows-x64.zip"),
    );
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("unsafe ZIP")
            .starts_with("Portable ZIP contains unsafe entry:")
    );
}

#[test]
fn rejects_invalid_setup_and_unexpected_runtime_or_directory() {
    let fixture = Fixture::valid();
    fs::write(
        fixture.artifact_root.join("AskBridge-9.9.9-Setup.exe"),
        [0_u8; 132],
    )
    .expect("invalid setup");
    assert_eq!(
        validate_package_artifacts(&fixture.options).expect_err("invalid setup"),
        "Setup EXE does not have the expected PE DOS header."
    );

    let fixture = Fixture::valid();
    fs::write(fixture.portable_root.join("chrome.exe"), b"runtime").expect("runtime");
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("runtime")
            .starts_with("Package unexpectedly bundled external runtime files:")
    );

    let fixture = Fixture::valid();
    fs::create_dir(fixture.portable_root.join("cache")).expect("cache dir");
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("portable directory")
            .starts_with("Portable package must be flat; found directories:")
    );
}

#[test]
fn rejects_missing_payload_and_corrupt_zip() {
    let fixture = Fixture::valid();
    fs::remove_file(fixture.portable_root.join("README.md")).expect("remove payload");
    assert_eq!(
        validate_package_artifacts(&fixture.options).expect_err("missing payload"),
        "Portable package is missing README.md."
    );

    let fixture = Fixture::valid();
    fs::write(
        fixture
            .artifact_root
            .join("AskBridge-9.9.9-windows-x64.zip"),
        b"not a zip",
    )
    .expect("corrupt ZIP");
    assert_eq!(
        validate_package_artifacts(&fixture.options).expect_err("corrupt ZIP"),
        "Portable ZIP does not have the expected file header."
    );
}

#[test]
fn rejects_release_identity_source_identity_and_size_bounds() {
    let mut fixture = Fixture::valid();
    let other_release = fixture._root.path().join("other.exe");
    fs::write(&other_release, b"other").expect("other release");
    fixture.options.expected_release_exe_path = other_release;
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("release identity")
            .starts_with("Packaged askbridge.exe hash does not match")
    );

    let fixture = Fixture::valid();
    fs::write(fixture.source_root.join("README.md"), b"updated").expect("source update");
    assert_eq!(
        validate_package_artifacts(&fixture.options).expect_err("source identity"),
        "Packaged README.md hash does not match source README.md."
    );

    let mut fixture = Fixture::valid();
    fixture.options.max_release_exe_bytes = 1;
    assert!(
        validate_package_artifacts(&fixture.options)
            .expect_err("size bound")
            .starts_with("Release EXE is")
    );
}

fn write_metadata(portable: &Path, value: serde_json::Value) {
    fs::write(
        portable.join("package.json"),
        serde_json::to_vec_pretty(&value).expect("JSON"),
    )
    .expect("metadata");
}

fn write_zip(portable: &Path, path: &Path, replacement: Option<(&str, &[u8])>) {
    let file = File::create(path).expect("zip file");
    let mut zip = ZipWriter::new(file);
    let options = FileOptions::<()>::default().compression_method(CompressionMethod::Deflated);
    let mut names: Vec<_> = fs::read_dir(portable)
        .expect("portable entries")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    names.sort();
    for name in names {
        let name = name.to_string_lossy();
        zip.start_file(name.as_ref(), options).expect("zip entry");
        if replacement.is_some_and(|(replacement_name, _)| replacement_name == name) {
            zip.write_all(replacement.expect("replacement").1)
                .expect("replacement bytes");
        } else {
            zip.write_all(&fs::read(portable.join(name.as_ref())).expect("payload bytes"))
                .expect("zip bytes");
        }
    }
    zip.finish().expect("finish zip");
}

fn write_unsafe_zip(path: &Path) {
    let file = File::create(path).expect("zip file");
    let mut zip = ZipWriter::new(file);
    let options = FileOptions::<()>::default().compression_method(CompressionMethod::Stored);
    zip.start_file("../escape.txt", options)
        .expect("unsafe entry");
    zip.write_all(b"escape").expect("unsafe bytes");
    zip.finish().expect("finish unsafe zip");
}

fn write_manifest(
    artifact_root: &Path,
    portable: &Path,
    zip: &Path,
    setup: &Path,
    extra: &[(&str, &Path)],
) {
    let mut lines = vec![
        format!("{}  {}", sha256(zip).expect("zip hash"), file_name(zip)),
        format!(
            "{}  {}",
            sha256(setup).expect("setup hash"),
            file_name(setup)
        ),
        format!(
            "{}  askbridge.exe",
            sha256(&portable.join("askbridge.exe")).expect("exe hash")
        ),
    ];
    lines.extend(
        extra
            .iter()
            .map(|(name, path)| format!("{}  {name}", sha256(path).expect("extra hash"))),
    );
    fs::write(
        artifact_root.join("AskBridge-9.9.9-SHA256SUMS.txt"),
        lines.join("\r\n") + "\r\n",
    )
    .expect("manifest");
}

fn write_minimal_pe(path: &Path) {
    fs::write(path, minimal_pe_with_suffix(&[])).expect("minimal PE");
}

fn minimal_pe_with_suffix(suffix: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0_u8; 132];
    bytes[0] = 0x4D;
    bytes[1] = 0x5A;
    bytes[0x3C] = 0x80;
    bytes[0x80..0x84].copy_from_slice(&[0x50, 0x45, 0x00, 0x00]);
    bytes.extend_from_slice(suffix);
    bytes
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("file name")
        .to_string_lossy()
        .into_owned()
}
