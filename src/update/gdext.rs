use super::git_dep::{self, GitDepSpec};
use crate::paths::ProjectPaths;
use anyhow::{Context, Result};
use std::fs;

const GODOT_TABLE: &[&str] = &["dependencies", "godot"];

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct UpdateGdextArgs {
    #[cfg_attr(
        feature = "cli",
        arg(long, default_value = "https://github.com/godot-rust/gdext.git")
    )]
    pub repo_url: String,
    #[cfg_attr(feature = "cli", arg(long, default_value = "master"))]
    pub branch: String,
    #[cfg_attr(feature = "cli", arg(long))]
    pub dry_run: bool,
    #[cfg_attr(feature = "cli", arg(long))]
    pub skip_lockfile: bool,
    #[cfg_attr(feature = "cli", arg(long, default_value = "gdxtask"))]
    pub user_agent: String,
}

impl Default for UpdateGdextArgs {
    fn default() -> Self {
        Self {
            repo_url: "https://github.com/godot-rust/gdext.git".to_owned(),
            branch: "master".to_owned(),
            dry_run: false,
            skip_lockfile: false,
            user_agent: "gdxtask".to_owned(),
        }
    }
}

pub fn execute(paths: &ProjectPaths, args: UpdateGdextArgs) -> Result<()> {
    let cargo_toml_path = paths.rust_dir.join("Cargo.toml");
    let original = fs::read_to_string(&cargo_toml_path)
        .with_context(|| format!("failed to read {}", cargo_toml_path.display()))?;
    let spec = godot_spec(args.user_agent.clone());
    let current = git_dep::read_rev(&original, &spec)?;
    let latest = git_dep::fetch_latest_rev(&args.repo_url, &args.branch, &args.user_agent)?;

    if current.as_deref() == Some(latest.as_str()) {
        println!("godot-rust is already at {latest}");
        return Ok(());
    }

    let updated = git_dep::set_rev(&original, &spec, &latest)?;
    let current_label = current.as_deref().unwrap_or("unpinned");
    if args.dry_run {
        println!("would update godot-rust from {current_label} to {latest}");
        return Ok(());
    }

    fs::write(&cargo_toml_path, updated)
        .with_context(|| format!("failed to write {}", cargo_toml_path.display()))?;

    if !args.skip_lockfile {
        crate::process::run(
            "cargo",
            &paths.rust_dir,
            &[
                "update".into(),
                "-p".into(),
                "godot".into(),
                "--precise".into(),
                latest,
            ],
        )?;
    }

    Ok(())
}

pub fn godot_spec(user_agent: impl Into<String>) -> GitDepSpec {
    GitDepSpec::new(GODOT_TABLE, user_agent)
}

pub fn read_godot_rev(cargo_toml: &str) -> Result<Option<String>> {
    git_dep::read_rev(cargo_toml, &godot_spec("gdxtask"))
}

pub fn set_godot_rev(cargo_toml: &str, rev: &str) -> Result<String> {
    git_dep::set_rev(cargo_toml, &godot_spec("gdxtask"), rev)
}

pub fn fetch_latest_rev(repo_url: &str, branch: &str, user_agent: &str) -> Result<String> {
    git_dep::fetch_latest_rev(repo_url, branch, user_agent)
}

pub use git_dep::{github_api_commit_url, is_sha};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_and_reads_godot_inline_table_rev() {
        let input = "[dependencies]\ngodot = { git = \"https://github.com/godot-rust/gdext\" }\n";
        let rev = "2222222222222222222222222222222222222222";
        let output = set_godot_rev(input, rev).unwrap();

        assert!(output.contains("rev = \"2222222222222222222222222222222222222222\""));
        assert_eq!(read_godot_rev(&output).unwrap().as_deref(), Some(rev));
    }
}
