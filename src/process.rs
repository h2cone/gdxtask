use anyhow::{Context, Result};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program {
    path: PathBuf,
}

impl Program {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl From<&str> for Program {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Program {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<PathBuf> for Program {
    fn from(value: PathBuf) -> Self {
        Self::new(value)
    }
}

impl From<&Path> for Program {
    fn from(value: &Path) -> Self {
        Self::new(value)
    }
}

/// Run a program with inherited stdio.
pub fn run(program: impl Into<Program>, cwd: &Path, args: &[String]) -> Result<()> {
    let program = program.into();
    let status = Command::new(program.path())
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to start {}", program.path().display()))?;

    if status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} {} failed with status {}",
            program.path().display(),
            args.join(" "),
            status
        )
    }
}

/// Run a program and capture stdout, including stderr in errors.
pub fn capture_stdout(program: impl Into<Program>, cwd: &Path, args: &[&str]) -> Result<String> {
    let program = program.into();
    let output = Command::new(program.path())
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to start {}", program.path().display()))?;

    if !output.status.success() {
        anyhow::bail!(
            "{} {} failed with status {}\nstderr:\n{}",
            program.path().display(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Convenience wrapper for `cargo <args...>` in `cwd`.
pub fn run_cargo(cwd: &Path, args: &[&str]) -> Result<()> {
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    run("cargo", cwd, &args)
}

pub fn string_args<const N: usize>(args: [&str; N]) -> Vec<String> {
    args.into_iter().map(String::from).collect()
}

pub fn push_os_arg(args: &mut Vec<String>, value: impl AsRef<OsStr>) {
    args.push(value.as_ref().to_string_lossy().into_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_program_path() {
        let program = Program::new("godot");
        assert_eq!(program.path(), Path::new("godot"));
    }

    #[test]
    fn accepts_pathbuf_as_program() {
        let program: Program = PathBuf::from("cargo").into();
        assert_eq!(program.path(), Path::new("cargo"));
    }
}
