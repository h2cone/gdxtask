use crate::{
    cli::BuildMode,
    godot,
    paths::ProjectPaths,
    process::{self, Program},
};
use anyhow::Result;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct RunArgs {
    #[cfg_attr(feature = "cli", arg(long, value_enum, default_value_t = BuildMode::Debug))]
    pub build: BuildMode,
    #[cfg_attr(feature = "cli", arg(long, default_value = "godot"))]
    pub godot_exe: String,
    #[cfg_attr(feature = "cli", arg(long))]
    pub editor: bool,
    #[cfg_attr(feature = "cli", arg(long))]
    pub headless: bool,
    #[cfg_attr(feature = "cli", arg(last = true))]
    pub godot_args: Vec<String>,
}

pub fn rust_build_args(build: BuildMode) -> Vec<Vec<String>> {
    match build {
        BuildMode::Debug => vec![vec!["build".into(), "--locked".into()]],
        BuildMode::Release => vec![vec!["build".into(), "--release".into(), "--locked".into()]],
        BuildMode::Both => vec![
            vec!["build".into(), "--release".into(), "--locked".into()],
            vec!["build".into(), "--locked".into()],
        ],
        BuildMode::None => Vec::new(),
    }
}

pub fn godot_launch_args(godot_dir: &Path, args: &RunArgs) -> Vec<String> {
    let mut launch = Vec::new();
    if args.headless {
        launch.push("--headless".into());
    }
    launch.push("--path".into());
    launch.push(godot_dir.to_string_lossy().into_owned());
    if args.editor {
        launch.push("--editor".into());
    }
    launch.extend(args.godot_args.clone());
    launch
}

pub fn execute(paths: &ProjectPaths, args: RunArgs) -> Result<()> {
    for cargo_args in rust_build_args(args.build) {
        process::run("cargo", &paths.rust_dir, &cargo_args)?;
    }

    let godot_exe = godot::resolve_godot_executable(&args.godot_exe)?;
    godot::normalize_extension_list(&paths.godot_dir)?;
    if godot::import_needed(&paths.godot_dir) {
        process::run(
            Program::new(godot_exe.clone()),
            &paths.root,
            &[
                "--path".into(),
                paths.godot_dir.to_string_lossy().into_owned(),
                "--import".into(),
                "--quit".into(),
            ],
        )?;
        godot::normalize_extension_list(&paths.godot_dir)?;
    }

    let launch_args = godot_launch_args(&paths.godot_dir, &args);
    process::run(Program::new(godot_exe), &paths.root, &launch_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_build_uses_release_flag_and_lockfile() {
        assert_eq!(
            rust_build_args(BuildMode::Release),
            vec![vec!["build", "--release", "--locked"]]
        );
    }

    #[test]
    fn both_builds_release_before_debug() {
        assert_eq!(
            rust_build_args(BuildMode::Both),
            vec![
                vec!["build", "--release", "--locked"],
                vec!["build", "--locked"]
            ]
        );
    }

    #[test]
    fn launch_args_include_real_godot_path() {
        let args = RunArgs {
            build: BuildMode::None,
            godot_exe: "godot".to_owned(),
            editor: true,
            headless: true,
            godot_args: vec!["--quit".to_owned()],
        };

        assert_eq!(
            godot_launch_args(Path::new("godot"), &args),
            vec!["--headless", "--path", "godot", "--editor", "--quit"]
        );
    }
}
