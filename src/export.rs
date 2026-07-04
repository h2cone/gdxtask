use crate::{
    cli::ExportTarget,
    godot,
    paths::ProjectPaths,
    process::{self, Program},
};
use anyhow::{Context, Result};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct ExportArgs {
    #[cfg_attr(feature = "cli", arg(long, value_enum, default_value_t = ExportTarget::Windows))]
    pub target: ExportTarget,
    #[cfg_attr(feature = "cli", arg(long, default_value = "godot"))]
    pub godot_exe: String,
    #[cfg_attr(feature = "cli", arg(long, default_value = "Windows Desktop"))]
    pub preset_name: String,
    #[cfg_attr(feature = "cli", arg(long, default_value = "game"))]
    pub product_name: String,
    #[cfg_attr(feature = "cli", arg(long))]
    pub force_create_export_preset: bool,
    #[cfg_attr(feature = "cli", arg(long))]
    pub skip_debug_build: bool,
}

pub fn export_output_path(
    paths: &ProjectPaths,
    target: ExportTarget,
    product_name: &str,
) -> PathBuf {
    let file_name = match target {
        ExportTarget::Windows => format!("{product_name}.exe"),
        ExportTarget::Macos => format!("{product_name}.zip"),
    };
    paths.export_dir.join(file_name)
}

pub fn export_platform(target: ExportTarget) -> &'static str {
    match target {
        ExportTarget::Windows => "Windows Desktop",
        ExportTarget::Macos => "macOS",
    }
}

pub fn rust_build_args(skip_debug_build: bool) -> Vec<Vec<String>> {
    let mut args = vec![vec!["build".into(), "--release".into(), "--locked".into()]];
    if !skip_debug_build {
        args.push(vec!["build".into(), "--locked".into()]);
    }
    args
}

pub fn ensure_export_preset(
    godot_dir: &std::path::Path,
    preset_name: &str,
    target: ExportTarget,
    force: bool,
) -> Result<()> {
    let path = godot_dir.join("export_presets.cfg");
    if path.is_file() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if content.contains(&format!("name=\"{preset_name}\"")) {
            return Ok(());
        }
        if !force {
            anyhow::bail!(
                "export preset '{preset_name}' not found in {}; pass --force-create-export-preset",
                path.display()
            );
        }
    }

    let content = format!(
        "[preset.0]\nname=\"{preset_name}\"\nplatform=\"{}\"\nrunnable=true\nadvanced_options=false\n\n",
        export_platform(target)
    );
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn execute(paths: &ProjectPaths, args: ExportArgs) -> Result<()> {
    for cargo_args in rust_build_args(args.skip_debug_build) {
        process::run("cargo", &paths.rust_dir, &cargo_args)?;
    }

    fs::create_dir_all(&paths.export_dir)
        .with_context(|| format!("failed to create {}", paths.export_dir.display()))?;
    ensure_export_preset(
        &paths.godot_dir,
        &args.preset_name,
        args.target,
        args.force_create_export_preset,
    )?;
    godot::normalize_extension_list(&paths.godot_dir)?;

    let godot_exe = godot::resolve_godot_executable(&args.godot_exe)?;
    let output = export_output_path(paths, args.target, &args.product_name);
    process::run(
        Program::new(godot_exe),
        &paths.root,
        &[
            "--headless".into(),
            "--path".into(),
            paths.godot_dir.to_string_lossy().into_owned(),
            "--export-release".into(),
            args.preset_name,
            output.to_string_lossy().into_owned(),
        ],
    )?;

    if output.exists() {
        Ok(())
    } else {
        anyhow::bail!("export failed: output not found at {}", output.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{paths::Layout, paths::ProjectPaths};
    use tempfile::TempDir;

    fn paths(root: &std::path::Path) -> ProjectPaths {
        ProjectPaths::from_root_unchecked(root.to_path_buf(), &Layout::default())
    }

    #[test]
    fn export_uses_product_name_and_target_extension() {
        let temp = TempDir::new().unwrap();
        let paths = paths(temp.path());
        assert_eq!(
            export_output_path(&paths, ExportTarget::Windows, "p1proto"),
            temp.path().join("export/p1proto.exe")
        );
        assert_eq!(
            export_output_path(&paths, ExportTarget::Macos, "p1proto"),
            temp.path().join("export/p1proto.zip")
        );
    }

    #[test]
    fn release_build_is_always_first() {
        assert_eq!(
            rust_build_args(false),
            vec![
                vec!["build", "--release", "--locked"],
                vec!["build", "--locked"]
            ]
        );
        assert_eq!(
            rust_build_args(true),
            vec![vec!["build", "--release", "--locked"]]
        );
    }

    #[test]
    fn force_creates_export_presets_file_for_target_platform() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("godot")).unwrap();
        ensure_export_preset(
            &temp.path().join("godot"),
            "macOS",
            ExportTarget::Macos,
            true,
        )
        .unwrap();
        let content = fs::read_to_string(temp.path().join("godot/export_presets.cfg")).unwrap();
        assert!(content.contains("name=\"macOS\""));
        assert!(content.contains("platform=\"macOS\""));
    }
}
