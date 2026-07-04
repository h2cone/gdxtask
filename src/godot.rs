use anyhow::{Context, Result};
use std::{
    env, fs,
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

/// Resolve a Godot executable, preferring the Windows console sibling when it is present.
pub fn resolve_godot_executable(requested: &str) -> Result<PathBuf> {
    let resolved = resolve_command(requested)?;
    if !cfg!(windows) {
        return Ok(resolved);
    }

    let dir = resolved.parent().unwrap_or_else(|| Path::new(""));
    let stem = resolved
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(requested);
    let ext = resolved
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");

    if stem.to_ascii_lowercase().ends_with("_console") {
        return Ok(resolved);
    }

    let sibling = if ext.is_empty() {
        dir.join(format!("{stem}_console"))
    } else {
        dir.join(format!("{stem}_console.{ext}"))
    };

    if sibling.is_file() {
        Ok(sibling)
    } else {
        Ok(resolved)
    }
}

/// Resolve a command from an absolute/relative path or from PATH.
pub fn resolve_command(command: &str) -> Result<PathBuf> {
    let path = Path::new(command);
    if path.components().count() > 1 || path.is_absolute() {
        if path.exists() {
            return Ok(path.to_path_buf());
        }
        anyhow::bail!("command not found: {command}");
    }

    let path_var = env::var_os("PATH").context("PATH is not set")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return Ok(candidate);
        }
        if cfg!(windows) {
            let exe = dir.join(format!("{command}.exe"));
            if exe.is_file() {
                return Ok(exe);
            }
        }
    }

    anyhow::bail!("command not found: {command}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
