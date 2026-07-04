use anyhow::{Context, Result};
use std::{
    env,
    path::{Path, PathBuf},
};

/// Conventional project layout under a repository root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layout {
    /// Rust/GDExtension crate directory under the repository root.
    pub rust_subdir: PathBuf,
    /// Godot project directory under the repository root.
    pub godot_subdir: PathBuf,
    /// Export output directory under the repository root.
    pub export_subdir: PathBuf,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            rust_subdir: "rust".into(),
            godot_subdir: "godot".into(),
            export_subdir: "export".into(),
        }
    }
}

/// Files/directories to require when validating a discovered layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Validate {
    pub rust_cargo_toml: bool,
    pub godot_project: bool,
    pub godot_addons_dir: bool,
}

impl Default for Validate {
    fn default() -> Self {
        Self {
            rust_cargo_toml: true,
            godot_project: true,
            godot_addons_dir: false,
        }
    }
}

/// Paths used by the reusable xtask commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectPaths {
    pub root: PathBuf,
    pub rust_dir: PathBuf,
    pub godot_dir: PathBuf,
    pub export_dir: PathBuf,
}

impl ProjectPaths {
    /// Discover paths from the process working directory, falling back to the
    /// current crate's manifest parent for direct tests of the library itself.
    ///
    /// In a normal `.cargo/config.toml` alias, `cargo xtask` is invoked from the
    /// repository root, so the working-directory probe is the one consumers use.
    pub fn discover(layout: &Layout) -> Result<Self> {
        let mut errors = Vec::new();
        let cwd = env::current_dir().context("failed to read current directory")?;
        for candidate in [cwd.clone(), cwd.parent().unwrap_or(&cwd).to_path_buf()] {
            match Self::from_root(candidate.clone(), layout) {
                Ok(paths) => return Ok(paths),
                Err(error) => errors.push(format!("{}: {error:#}", candidate.display())),
            }
        }

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let fallback = manifest_dir
            .parent()
            .context("CARGO_MANIFEST_DIR has no parent")?
            .to_path_buf();
        Self::from_root(fallback.clone(), layout).with_context(|| {
            format!(
                "failed to discover project layout; attempts:\n{}",
                errors.join("\n")
            )
        })
    }

    /// Construct paths from an explicit repository root and validate the default
    /// Godot + Rust project files.
    pub fn from_root(root: PathBuf, layout: &Layout) -> Result<Self> {
        let paths = Self::from_root_unchecked(root, layout);
        paths.validate(&Validate::default())?;
        Ok(paths)
    }

    /// Construct paths without validation.
    pub fn from_root_unchecked(root: PathBuf, layout: &Layout) -> Self {
        Self {
            rust_dir: root.join(&layout.rust_subdir),
            godot_dir: root.join(&layout.godot_subdir),
            export_dir: root.join(&layout.export_subdir),
            root,
        }
    }

    /// Validate the selected files/directories and return `self` for chaining.
    pub fn validate(&self, validate: &Validate) -> Result<&Self> {
        if validate.rust_cargo_toml {
            require_file(&self.rust_dir.join("Cargo.toml"))?;
        }
        if validate.godot_project {
            require_file(&self.godot_dir.join("project.godot"))?;
        }
        if validate.godot_addons_dir {
            require_dir(&self.godot_dir.join("addons"))?;
        }
        Ok(self)
    }
}

fn require_file(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        anyhow::bail!("required file not found: {}", path.display())
    }
}

fn require_dir(path: &Path) -> Result<()> {
    if path.is_dir() {
        Ok(())
    } else {
        anyhow::bail!("required directory not found: {}", path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn builds_standard_project_layout() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("rust")).unwrap();
        fs::create_dir(temp.path().join("godot")).unwrap();
        fs::write(
            temp.path().join("rust/Cargo.toml"),
            "[package]\nname='rust'\n",
        )
        .unwrap();
        fs::write(temp.path().join("godot/project.godot"), "; godot\n").unwrap();

        let paths = ProjectPaths::from_root(temp.path().to_path_buf(), &Layout::default()).unwrap();

        assert_eq!(paths.root, temp.path());
        assert_eq!(paths.rust_dir, temp.path().join("rust"));
        assert_eq!(paths.godot_dir, temp.path().join("godot"));
        assert_eq!(paths.export_dir, temp.path().join("export"));
    }

    #[test]
    fn optional_addons_validation_is_explicit() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("rust")).unwrap();
        fs::create_dir_all(temp.path().join("godot")).unwrap();
        fs::write(temp.path().join("rust/Cargo.toml"), "").unwrap();
        fs::write(temp.path().join("godot/project.godot"), "").unwrap();
        let paths = ProjectPaths::from_root(temp.path().to_path_buf(), &Layout::default()).unwrap();

        assert!(
            paths
                .validate(&Validate {
                    godot_addons_dir: true,
                    ..Validate::default()
                })
                .is_err()
        );
    }
}
