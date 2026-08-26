use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=askbridge.rc");
    let app_icon = manifest_dir().join("../../assets/branding/askbridge-final/icons/askbridge.ico");
    let app_icon = app_icon.canonicalize().expect("app icon path resolves");
    println!("cargo:rerun-if-changed={}", app_icon.display());

    let target = env::var("TARGET").expect("Cargo provides TARGET");
    if target.ends_with("windows-gnu") {
        let output =
            PathBuf::from(env::var_os("OUT_DIR").expect("out dir")).join("askbridge-resource.o");
        let status = Command::new("windres")
            .current_dir(manifest_dir())
            .args([
                "--input",
                "askbridge.rc",
                "--output-format",
                "coff",
                "--output",
            ])
            .arg(&output)
            .status()
            .expect("windres is required for the Windows GNU build");
        assert!(
            status.success(),
            "windres failed to embed the application manifest and icon"
        );
        println!("cargo:rustc-link-arg-bin=askbridge={}", output.display());
        println!(
            "cargo:rustc-link-arg-bin=askbridge-setup={}",
            output.display()
        );
    } else if target.ends_with("windows-msvc") {
        // The .rc embeds both the manifest (RT_MANIFEST, keeps askbridge-setup.exe
        // out of UAC installer detection) and the application icon (RT_GROUP_ICON).
        let output = compile_msvc_resources();
        println!("cargo:rustc-link-arg-bin=askbridge={}", output.display());
        println!(
            "cargo:rustc-link-arg-bin=askbridge-setup={}",
            output.display()
        );
    }

    if target.contains("windows") {
        copy_webview2_loader(&target);
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"))
}

fn compile_msvc_resources() -> PathBuf {
    let rc_exe = find_rc_exe().unwrap_or_else(|| {
        panic!(
            "rc.exe (Windows SDK resource compiler) is required for the Windows MSVC build; \
             install the Windows SDK or run cargo from a Developer Command Prompt"
        )
    });
    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("out dir")).join("askbridge-resource.res");
    let status = Command::new(&rc_exe)
        .args(["/nologo", "/fo"])
        .arg(&output)
        .arg(manifest_dir().join("askbridge.rc"))
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", rc_exe.display()));
    assert!(
        status.success(),
        "rc.exe failed to compile askbridge.rc (manifest and icon)"
    );
    output
}

fn find_rc_exe() -> Option<PathBuf> {
    // Developer Command Prompts put rc.exe on PATH.
    if let Ok(output) = Command::new("where").arg("rc.exe").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(first) = stdout.lines().next().map(str::trim) {
                if !first.is_empty() {
                    return Some(PathBuf::from(first));
                }
            }
        }
    }
    // GitHub runners and plain SDK installs keep it under Windows Kits\10\bin,
    // one folder per SDK version, with a host-specific subfolder.
    let mut candidates: Vec<(String, u8, PathBuf)> = Vec::new();
    let roots = [
        env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        env::var_os("ProgramFiles").map(PathBuf::from),
    ];
    for root in roots.into_iter().flatten() {
        let bin = root.join("Windows Kits").join("10").join("bin");
        let Ok(entries) = fs::read_dir(&bin) else {
            continue;
        };
        for entry in entries.flatten() {
            let version = entry.file_name();
            let Some(version) = version.to_str() else {
                continue;
            };
            if !version.starts_with("10.") {
                continue;
            }
            for (host, preference) in [("x64", 1u8), ("x86", 0u8)] {
                let rc = entry.path().join(host).join("rc.exe");
                if rc.is_file() {
                    candidates.push((version.to_owned(), preference, rc));
                }
            }
        }
    }
    // Highest SDK version wins; prefer the x64 host compiler within a version.
    candidates.sort();
    candidates.pop().map(|(_, _, path)| path)
}

fn copy_webview2_loader(target: &str) {
    let Some(source) = find_webview2_loader(target) else {
        println!("cargo:warning=WebView2Loader.dll was not found; runtime WebView UI may fail");
        return;
    };
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("out dir"));
    let Some(profile_dir) = out_dir.ancestors().nth(3).map(Path::to_path_buf) else {
        println!("cargo:warning=unable to resolve Cargo profile output directory");
        return;
    };
    let destination = profile_dir.join("WebView2Loader.dll");
    if let Err(error) = fs::copy(&source, &destination) {
        println!(
            "cargo:warning=failed to copy {} to {}: {}",
            source.display(),
            destination.display(),
            error
        );
    }
}

fn find_webview2_loader(target: &str) -> Option<PathBuf> {
    let arch = if target.contains("x86_64") {
        "x64"
    } else if target.contains("i686") {
        "x86"
    } else if target.contains("aarch64") {
        "arm64"
    } else {
        return None;
    };
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".cargo")))?;
    let registry_src = cargo_home.join("registry").join("src");
    let mut candidates = Vec::new();
    for registry in fs::read_dir(registry_src).ok()? {
        let registry = registry.ok()?;
        let entries = fs::read_dir(registry.path()).ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("webview2-com-sys-") {
                let loader = entry.path().join(arch).join("WebView2Loader.dll");
                if loader.is_file() {
                    candidates.push(loader);
                }
            }
        }
    }
    candidates.sort();
    candidates.pop()
}
