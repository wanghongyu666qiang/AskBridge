use std::{
    env,
    path::{Component, Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone)]
pub(crate) struct PackageOptions {
    artifact_root: PathBuf,
}

impl PackageOptions {
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut artifact_root = None;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--artifact-root" if artifact_root.is_none() => {
                    artifact_root = Some(validate_artifact_root(PathBuf::from(value))?);
                }
                "--artifact-root" => {
                    return Err("--artifact-root may only be specified once".to_owned());
                }
                _ => return Err(format!("unknown option '{flag}'")),
            }
        }

        Ok(Self {
            artifact_root: artifact_root
                .ok_or_else(|| "--artifact-root is required for packaging".to_owned())?,
        })
    }
}

pub(crate) fn package(options: &PackageOptions) -> Result<(), String> {
    let repo_root = repository_root()?;
    let script = repo_root.join("scripts/package.ps1");
    if !script.is_file() {
        return Err(format!(
            "package script is unavailable: {}",
            script.display()
        ));
    }

    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-ArtifactRoot")
        .arg(&options.artifact_root)
        .current_dir(&repo_root)
        .status()
        .map_err(|error| format!("starting package script: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("package script failed with status {status}"))
    }
}

fn validate_artifact_root(path: PathBuf) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("ArtifactRoot must be an explicit absolute path.".to_owned());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err("ArtifactRoot must not contain '.' or '..' path components.".to_owned());
    }
    Ok(path)
}

fn repository_root() -> Result<PathBuf, String> {
    let current =
        env::current_dir().map_err(|error| format!("reading current directory: {error}"))?;
    current
        .ancestors()
        .find(|candidate| {
            candidate.join("Cargo.toml").is_file()
                && candidate.join("scripts/package.ps1").is_file()
        })
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "could not locate AskBridge repository root from {}",
                current.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_root_must_be_absolute_and_lexically_safe() {
        assert!(validate_artifact_root(PathBuf::from("relative")).is_err());
        let root = env::temp_dir().join("askbridge-package-safe");
        assert_eq!(validate_artifact_root(root.clone()).expect("root"), root);
    }

    #[test]
    fn artifact_root_rejects_parent_aliases() {
        let unsafe_root = env::temp_dir()
            .join("askbridge-package-parent")
            .join("..")
            .join("output");
        let error = validate_artifact_root(unsafe_root).expect_err("parent alias");
        assert!(error.contains("must not contain"));
    }
}
