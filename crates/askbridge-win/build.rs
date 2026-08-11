use std::{env, path::PathBuf, process::Command};

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
    } else if target.ends_with("windows-msvc") {
        let manifest = manifest_dir.join("app.manifest");
        println!("cargo:rustc-link-arg-bin=askbridge=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=askbridge=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
