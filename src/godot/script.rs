//! Headless script command construction and execution.

use super::OutputPolicy;
use anyhow::{Context, Result};
use std::{path::Path, process::Command};

#[derive(Clone, Debug, Default)]
pub struct ScriptOptions<'a> {
    pub user_args: &'a [&'a str],
    pub editor: bool,
    /// Frame limit for scripts that complete synchronously. Leave unset for
    /// asynchronous scripts; one frame is not a wall-clock timeout.
    pub quit_after: Option<u32>,
    pub output: OutputPolicy<'a>,
}

/// Construct a command so callers can choose their own logging or supervision.
pub fn script_command(
    project: &Path,
    executable: &Path,
    script: &str,
    options: &ScriptOptions<'_>,
) -> Command {
    let mut command = Command::new(executable);
    command.args(["--headless", "--path"]).arg(project);
    if options.editor {
        command.arg("--editor");
    }
    if let Some(frames) = options.quit_after {
        command.arg("--quit-after").arg(frames.to_string());
    }
    command.args(["--script", script]);
    if !options.user_args.is_empty() {
        command.arg("--").args(options.user_args);
    }
    command
}

pub fn run_script(
    project: &Path,
    executable: &Path,
    script: &str,
    label: &str,
    options: &ScriptOptions<'_>,
) -> Result<()> {
    let output = script_command(project, executable, script, options)
        .output()
        .with_context(|| format!("run Godot script {script}"))?;
    options.output.ensure_success(&output, label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_options_preserve_engine_and_user_argument_boundaries() {
        let options = ScriptOptions {
            user_args: &["--fixture"],
            quit_after: Some(1),
            editor: true,
            ..Default::default()
        };
        let command = script_command(
            Path::new("project with spaces"),
            Path::new("godot"),
            "res://fixture.gd",
            &options,
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--headless",
                "--path",
                "project with spaces",
                "--editor",
                "--quit-after",
                "1",
                "--script",
                "res://fixture.gd",
                "--",
                "--fixture"
            ]
        );
        let command = script_command(
            Path::new("project"),
            Path::new("godot"),
            "res://fixture.gd",
            &ScriptOptions::default(),
        );
        assert!(
            !command
                .get_args()
                .any(|arg| arg == "--" || arg == "--quit-after")
        );
    }
}
