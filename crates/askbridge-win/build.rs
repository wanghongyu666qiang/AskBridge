use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=askbridge.rc");

    let target = env::var("TARGET").expect("Cargo provides TARGET");
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    if target.ends_with("windows-gnu") {
        let output =
            PathBuf::from(env::var_os("OUT_DIR").expect("out dir")).join("askbridge-resource.o");
        let status = Command::new("windres")
            .current_dir(&manifest_dir)
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
            "windres failed to embed the application manifest"
        );
        println!("cargo:rustc-link-arg-bin=askbridge={}", output.display());
        println!(
            "cargo:rustc-link-arg-bin=askbridge-setup={}",
            output.display()
        );
    } else if target.ends_with("windows-msvc") {
        let manifest = manifest_dir.join("app.manifest");
        println!("cargo:rustc-link-arg-bin=askbridge=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=askbridge=/MANIFESTINPUT:{}",
            manifest.display()
        );
        println!("cargo:rustc-link-arg-bin=askbridge-setup=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=askbridge-setup=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }

    if target.contains("windows") {
        copy_webview2_loader(&target);
    }
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
