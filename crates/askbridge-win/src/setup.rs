#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("askbridge-setup supports Windows only");

use std::{
    env, fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const TRAILER_MAGIC: &[u8; 16] = b"ASKBRIDGESETUP10";
const FOOTER_LEN: u64 = 24;

#[derive(Debug)]
struct PayloadEntry {
    name: String,
    offset: u64,
    length: u64,
}

fn main() {
    if let Err(error) = run() {
        show_error("AskBridge Setup failed", &error.to_string());
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let setup_exe = env::current_exe()?;
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
        Command::new("powershell.exe")
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&install_script)
            .current_dir(&extraction_root)
            .output()
    })();

    let cleanup_result = fs::remove_dir_all(&extraction_root);
    let output = install_status?;
    if let Err(error) = cleanup_result {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error);
        }
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
        .status();
}
