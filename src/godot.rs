mod output;
mod script;

pub use output::OutputPolicy;
pub use script::{ScriptOptions, run_script, script_command};

use anyhow::{Context, Result};
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

/// Normalize Godot's extension cache so `res://rust.gdextension` is present and
/// stale `res://` entries are removed.
pub fn normalize_extension_list(godot_dir: &Path) -> Result<()> {
    let cache_dir = godot_dir.join(".godot");
    let list_path = cache_dir.join("extension_list.cfg");
    let default_ext = "res://rust.gdextension";

    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create {}", cache_dir.display()))?;

    let mut kept = Vec::new();
    if list_path.is_file() {
        let raw = fs::read_to_string(&list_path)
            .with_context(|| format!("failed to read {}", list_path.display()))?;
        for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let Some(relative) = line.strip_prefix("res://") else {
                continue;
            };
            if godot_dir.join(relative).exists() {
                kept.push(line.to_owned());
            }
        }
    }

    if godot_dir.join("rust.gdextension").exists() && !kept.iter().any(|line| line == default_ext) {
        kept.insert(0, default_ext.to_owned());
    }

    fs::write(&list_path, kept.join("\n"))
        .with_context(|| format!("failed to write {}", list_path.display()))?;

    Ok(())
}

/// Return true when the Godot import cache is missing or empty.
pub fn import_needed(godot_dir: &Path) -> bool {
    let imported = godot_dir.join(".godot/imported");
    if !imported.is_dir() {
        return true;
    }

    match fs::read_dir(imported) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => true,
    }
}

/// Resolve an explicitly requested executable. An empty string selects the
/// standard environment/PATH search. Nonempty values always take precedence.
pub fn resolve_godot_executable(requested: &str) -> Result<PathBuf> {
    find_godot((!requested.is_empty()).then(|| Path::new(requested)))
}

/// Search explicit path/name, GODOT4_BIN, GODOT_BIN, then godot4/godot on PATH.
/// A configured but invalid executable fails instead of silently falling back.
/// Windows uses an existing `_console` sibling to retain diagnostic output.
pub fn find_godot(explicit: Option<&Path>) -> Result<PathBuf> {
    find_godot_with(explicit, |name| env::var_os(name), resolve_os_command)
}

fn find_godot_with(
    explicit: Option<&Path>,
    mut environment: impl FnMut(&str) -> Option<OsString>,
    mut resolve: impl FnMut(&OsStr) -> Result<PathBuf>,
) -> Result<PathBuf> {
    let configured = explicit
        .map(|path| path.as_os_str().to_owned())
        .or_else(|| environment("GODOT4_BIN"))
        .or_else(|| environment("GODOT_BIN"));
    let resolved = if let Some(command) = configured {
        resolve(&command)?
    } else {
        ["godot4", "godot"].into_iter()
            .find_map(|name| resolve(OsStr::new(name)).ok())
            .context("Godot was not found; pass --godot-exe, set GODOT4_BIN/GODOT_BIN, or add godot4/godot to PATH")?
    };
    Ok(prefer_console(resolved))
}

fn prefer_console(resolved: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return resolved;
    }
    let dir = resolved.parent().unwrap_or_else(|| Path::new(""));
    let Some(stem) = resolved.file_stem() else {
        return resolved;
    };
    if stem
        .to_string_lossy()
        .to_ascii_lowercase()
        .ends_with("_console")
    {
        return resolved;
    }
    let mut name = stem.to_os_string();
    name.push("_console");
    if let Some(extension) = resolved.extension() {
        name.push(".");
        name.push(extension);
    }
    let sibling = dir.join(name);
    if sibling.is_file() { sibling } else { resolved }
}

/// Resolve a command from a path or PATH, including executable/PATHEXT checks
/// for bare command names. Returned paths are absolute so changing cwd is safe.
pub fn resolve_command(command: &str) -> Result<PathBuf> {
    resolve_os_command(OsStr::new(command))
}

fn resolve_os_command(command: &OsStr) -> Result<PathBuf> {
    let path = Path::new(command);
    if path.components().count() > 1 || path.is_absolute() {
        anyhow::ensure!(path.is_file(), "command not found: {}", path.display());
        return std::path::absolute(path).context("make executable path absolute");
    }
    which::which(command).with_context(|| format!("command not found: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn executable_search_preserves_precedence_without_mutating_environment() {
        let explicit = Path::new("explicit-godot");
        let mut calls = Vec::new();
        let found = find_godot_with(
            Some(explicit),
            |_| panic!("explicit must bypass environment"),
            |name| {
                calls.push(name.to_owned());
                Ok(PathBuf::from(name))
            },
        )
        .unwrap();
        assert_eq!(found, explicit);
        assert_eq!(calls, [OsString::from("explicit-godot")]);

        for (first, second, expected) in [
            (Some("godot-four"), Some("godot-old"), "godot-four"),
            (None, Some("godot-old"), "godot-old"),
            (None, None, "godot4"),
        ] {
            let found = find_godot_with(
                None,
                |key| match key {
                    "GODOT4_BIN" => first.map(OsString::from),
                    "GODOT_BIN" => second.map(OsString::from),
                    _ => unreachable!(),
                },
                |name| Ok(PathBuf::from(name)),
            )
            .unwrap();
            assert_eq!(found, Path::new(expected));
        }
    }

    #[test]
    fn invalid_configuration_is_not_hidden_by_path_fallback() {
        let mut calls = Vec::new();
        let result = find_godot_with(
            None,
            |_| Some("missing".into()),
            |name| {
                calls.push(name.to_owned());
                anyhow::bail!("not found")
            },
        );
        assert!(result.is_err());
        assert_eq!(calls, [OsString::from("missing")]);
        let found = find_godot_with(
            None,
            |_| None,
            |name| {
                if name == "godot4" {
                    anyhow::bail!("not found");
                }
                Ok(PathBuf::from(name))
            },
        )
        .unwrap();
        assert_eq!(found, Path::new("godot"));
    }

    #[test]
    fn executable_paths_reject_directories_and_are_absolute() {
        let temp = TempDir::new().unwrap();
        assert!(resolve_godot_executable(temp.path().to_str().unwrap()).is_err());
        let executable = temp.path().join("custom-godot");
        fs::write(&executable, "").unwrap();
        assert!(find_godot(Some(&executable)).unwrap().is_absolute());
    }

    #[test]
    fn normalize_keeps_existing_res_paths_and_adds_default() {
        let temp = TempDir::new().unwrap();
        let godot = temp.path();
        fs::create_dir(godot.join(".godot")).unwrap();
        fs::write(godot.join("rust.gdextension"), "").unwrap();
        fs::create_dir(godot.join("entity")).unwrap();
        fs::write(godot.join("entity/example.gdextension"), "").unwrap();
        fs::write(
            godot.join(".godot/extension_list.cfg"),
            "res://missing.gdextension\nnot-res\nres://entity/example.gdextension\n",
        )
        .unwrap();

        normalize_extension_list(godot).unwrap();

        assert_eq!(
            fs::read_to_string(godot.join(".godot/extension_list.cfg")).unwrap(),
            "res://rust.gdextension\nres://entity/example.gdextension"
        );
    }

    #[test]
    fn import_is_needed_when_imported_dir_is_missing_or_empty() {
        let temp = TempDir::new().unwrap();
        assert!(import_needed(temp.path()));
        fs::create_dir_all(temp.path().join(".godot/imported")).unwrap();
        assert!(import_needed(temp.path()));
        fs::write(temp.path().join(".godot/imported/asset.import"), "").unwrap();
        assert!(!import_needed(temp.path()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefers_console_sibling_for_absolute_path() {
        let temp = TempDir::new().unwrap();
        let gui = temp.path().join("godot.exe");
        let console = temp.path().join("godot_console.exe");
        fs::write(&gui, "").unwrap();
        fs::write(&console, "").unwrap();

        assert_eq!(
            resolve_godot_executable(gui.to_str().unwrap()).unwrap(),
            console
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_uses_resolved_path_directly() {
        let temp = TempDir::new().unwrap();
        let exe = temp.path().join("godot");
        fs::write(&exe, "").unwrap();

        assert_eq!(
            resolve_godot_executable(exe.to_str().unwrap()).unwrap(),
            exe
        );
    }
}
