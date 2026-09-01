#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("askbridge-setup supports Windows only");

use std::{
    env, fs,
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const TRAILER_MAGIC: &[u8; 16] = b"ASKBRIDGESETUP10";
const FOOTER_LEN: u64 = 24;
const INSTALL_ROOT_ENV: &str = "ASKBRIDGE_INSTALL_ROOT";
const UPDATE_PARENT_PID_ENV: &str = "ASKBRIDGE_UPDATE_PARENT_PID";
const RESTART_AFTER_INSTALL_ENV: &str = "ASKBRIDGE_RESTART_AFTER_INSTALL";
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug)]
struct PayloadEntry {
    name: String,
    offset: u64,
    length: u64,
}

#[derive(Debug)]
struct UpdateLaunch {
    install_root: PathBuf,
    parent_pid: u32,
}

fn main() {
    if let Err(error) = run() {
        show_error("AskBridge Setup failed", &error.to_string());
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let setup_exe = env::current_exe()?;
    let update_launch = validate_update_environment()?;
    let entries = read_payload_manifest(&setup_exe)?;
    let extraction_root = unique_extraction_root()?;
    fs::create_dir_all(&extraction_root)?;
    let install_script = extraction_root.join("Install-AskBridge.ps1");

    let install_status = (|| -> io::Result<std::process::Output> {
        extract_payload(&setup_exe, &entries, &extraction_root)?;
        if !install_script.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "installer payload is missing Install-AskBridge.ps1",
            ));
        }
        let mut command = Command::new("powershell.exe");
        command
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&install_script)
            .creation_flags(CREATE_NO_WINDOW)
            .current_dir(&extraction_root);
        if let Some(update) = &update_launch {
            // Normalize and explicitly forward the update contract. This prevents a malformed
            // inherited value from reaching the installer script after Setup has validated it.
            command
                .arg("-InstallRoot")
                .arg(&update.install_root)
                .env(INSTALL_ROOT_ENV, &update.install_root)
                .env(UPDATE_PARENT_PID_ENV, update.parent_pid.to_string())
                .env(RESTART_AFTER_INSTALL_ENV, "1");
        }
        command.output()
    })();

    let cleanup_result = fs::remove_dir_all(&extraction_root);
    let output = install_status?;
    if let Err(error) = cleanup_result
        && error.kind() != io::ErrorKind::NotFound
    {
        return Err(error);
    }
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(io::Error::other(format!(
            "Install-AskBridge.ps1 exited with {}. stdout={} stderr={}",
            output.status,
            stdout.trim(),
            stderr.trim()
        )))
    }
}

fn validate_update_environment() -> io::Result<Option<UpdateLaunch>> {
    let parent_pid = match env::var(UPDATE_PARENT_PID_ENV) {
        Ok(value) => Some(parse_update_parent_pid(&value, std::process::id())?),
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ASKBRIDGE_UPDATE_PARENT_PID is not valid UTF-8",
            ));
        }
    };
    let restart_requested = match env::var(RESTART_AFTER_INSTALL_ENV) {
        Ok(value) => value == "1",
        Err(env::VarError::NotPresent) => false,
        Err(env::VarError::NotUnicode(_)) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ASKBRIDGE_RESTART_AFTER_INSTALL is not valid UTF-8",
            ));
        }
    };

    if parent_pid.is_none() && !restart_requested {
        return Ok(None);
    }
    let parent_pid = parent_pid.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_UPDATE_PARENT_PID is required for an update install",
        )
    })?;
    let requested_root = env::var(INSTALL_ROOT_ENV).map_err(|error| match error {
        env::VarError::NotPresent => io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_INSTALL_ROOT is required for an update install",
        ),
        env::VarError::NotUnicode(_) => io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_INSTALL_ROOT is not valid UTF-8",
        ),
    })?;
    let requested_root = PathBuf::from(requested_root);
    if !requested_root.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_INSTALL_ROOT must be an absolute path",
        ));
    }
    let canonical_install_root = fs::canonicalize(&requested_root).map_err(|source| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("ASKBRIDGE_INSTALL_ROOT cannot be resolved: {source}"),
        )
    })?;
    let install_root = powershell_compatible_path(&canonical_install_root)?;
    if install_root.file_name().is_none() || !install_root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_INSTALL_ROOT must name an existing non-root directory",
        ));
    }
    if !install_root.join("askbridge.exe").is_file()
        || !install_root.join("install-manifest.json").is_file()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_INSTALL_ROOT is not an existing AskBridge installation",
        ));
    }
    Ok(Some(UpdateLaunch {
        install_root,
        parent_pid,
    }))
}

fn powershell_compatible_path(path: &Path) -> io::Result<PathBuf> {
    let value = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_INSTALL_ROOT resolved to a non-Unicode path",
        )
    })?;
    let normalized = if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    };
    if !normalized.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_INSTALL_ROOT did not resolve to a PowerShell-compatible absolute path",
        ));
    }
    Ok(normalized)
}

fn parse_update_parent_pid(value: &str, current_pid: u32) -> io::Result<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_UPDATE_PARENT_PID must be a positive decimal process ID",
        ));
    }
    let pid = value.parse::<u64>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_UPDATE_PARENT_PID must be a positive decimal process ID",
        )
    })?;
    if pid == 0 || pid > u32::MAX as u64 || pid == current_pid as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ASKBRIDGE_UPDATE_PARENT_PID must identify another valid process",
        ));
    }
    Ok(pid as u32)
}

fn read_payload_manifest(path: &Path) -> io::Result<Vec<PayloadEntry>> {
    let mut file = fs::File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < FOOTER_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "setup executable is missing the AskBridge payload footer",
        ));
    }
    file.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
    let mut magic = [0u8; 16];
    file.read_exact(&mut magic)?;
    if &magic != TRAILER_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "setup executable is missing the AskBridge payload magic",
        ));
    }
    let manifest_len = read_u64(&mut file)?;
    if manifest_len == 0 || manifest_len > file_len - FOOTER_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "setup executable contains an invalid payload manifest length",
        ));
    }
    let manifest_offset = file_len - FOOTER_LEN - manifest_len;
    file.seek(SeekFrom::Start(manifest_offset))?;
    let mut manifest = vec![0u8; manifest_len as usize];
    file.read_exact(&mut manifest)?;
    parse_manifest(&manifest, manifest_offset)
}

fn parse_manifest(bytes: &[u8], manifest_offset: u64) -> io::Result<Vec<PayloadEntry>> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "payload manifest is not UTF-8"))?;
    let mut entries = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let name = parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "payload name missing"))?;
        let offset = parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "payload offset missing"))?
            .parse::<u64>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "payload offset is invalid"))?;
        let length = parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "payload length missing"))?
            .parse::<u64>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "payload length is invalid"))?;
        if !is_safe_payload_name(name) || offset + length > manifest_offset {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "payload manifest contains an unsafe entry",
            ));
        }
        entries.push(PayloadEntry {
            name: name.to_owned(),
            offset,
            length,
        });
    }
    if entries.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "payload manifest is empty",
        ));
    }
    Ok(entries)
}

fn extract_payload(
    setup_exe: &Path,
    entries: &[PayloadEntry],
    extraction_root: &Path,
) -> io::Result<()> {
    let mut source = fs::File::open(setup_exe)?;
    for entry in entries {
        source.seek(SeekFrom::Start(entry.offset))?;
        let mut remaining = entry.length;
        let target = extraction_root.join(&entry.name);
        let mut output = fs::File::create(target)?;
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let to_read = buffer.len().min(remaining as usize);
            source.read_exact(&mut buffer[..to_read])?;
            output.write_all(&buffer[..to_read])?;
            remaining -= to_read as u64;
        }
    }
    Ok(())
}

fn unique_extraction_root() -> io::Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| io::Error::other("system clock is before UNIX_EPOCH"))?
        .as_millis();
    Ok(env::temp_dir().join(format!("AskBridge-Setup-{}-{stamp}", std::process::id())))
}

fn is_safe_payload_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && !name.contains('/')
        && !name.contains(':')
        && name != "."
        && name != ".."
}

fn read_u64(reader: &mut fs::File) -> io::Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn show_error(title: &str, message: &str) {
    if env::var_os("ASKBRIDGE_SETUP_NO_DIALOG").is_some() {
        eprintln!("{title}: {message}");
        return;
    }
    let _ = Command::new("powershell.exe")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(format!(
            "Add-Type -AssemblyName PresentationFramework; [System.Windows.MessageBox]::Show({:?}, {:?}) | Out-Null",
            message, title
        ))
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_parent_pid_accepts_only_a_positive_decimal_pid() {
        assert_eq!(parse_update_parent_pid("1234", 4321).expect("pid"), 1234);
        for value in ["", "0", "-1", "+1", " 1", "1 ", "1.0", "abc"] {
            assert!(parse_update_parent_pid(value, 4321).is_err(), "{value:?}");
        }
    }

    #[test]
    fn update_parent_pid_cannot_be_setup_or_exceed_windows_pid_range() {
        assert!(parse_update_parent_pid("4321", 4321).is_err());
        assert!(parse_update_parent_pid("4294967296", 4321).is_err());
    }

    #[test]
    fn powershell_path_removes_windows_verbatim_prefixes() {
        assert_eq!(
            powershell_compatible_path(Path::new(r"\\?\D:\AskBridge")).expect("drive path"),
            PathBuf::from(r"D:\AskBridge")
        );
        assert_eq!(
            powershell_compatible_path(Path::new(r"\\?\UNC\server\share\AskBridge"))
                .expect("UNC path"),
            PathBuf::from(r"\\server\share\AskBridge")
        );
    }
}
