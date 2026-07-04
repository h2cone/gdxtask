use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use crate::paths::ProjectPaths;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadKind {
    /// Prefer the release's published `.zip` asset when one exists.
    ReleaseAssetZip,
    /// GitHub auto-generated zipball.
    Zipball,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddonSpec {
    pub label: &'static str,
    /// Repository in `owner/repo` form.
    pub repo: &'static str,
    /// Directory inside the archive that contains `plugin.cfg`.
    pub package_dir: &'static str,
    /// Path relative to `ProjectPaths::godot_dir`.
    pub target_dir: &'static str,
    pub download: DownloadKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddonSelection<'a> {
    All,
    One(&'a str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct UpdateGodotAddonsArgs {
    /// Pin to a specific git ref (tag or branch). Single-addon only.
    #[cfg_attr(feature = "cli", arg(long = "ref"))]
    pub ref_name: Option<String>,
    #[cfg_attr(feature = "cli", arg(long))]
    pub dry_run: bool,
    #[cfg_attr(feature = "cli", arg(long, default_value = "gdxtask"))]
    pub user_agent: String,
}

impl Default for UpdateGodotAddonsArgs {
    fn default() -> Self {
        Self {
            ref_name: None,
            dry_run: false,
            user_agent: "gdxtask".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Release {
    tag: String,
    download_url: String,
}

pub fn execute(
    paths: &ProjectPaths,
    specs: &[AddonSpec],
    selection: AddonSelection<'_>,
    args: UpdateGodotAddonsArgs,
) -> Result<()> {
    let addons = selected_addons(specs, selection)?;
    if args.ref_name.is_some() && addons.len() != 1 {
        anyhow::bail!("--ref can only be used with one selected addon");
    }

    for addon in addons {
        update_addon(
            paths,
            addon,
            args.ref_name.as_deref(),
            args.dry_run,
            &args.user_agent,
        )?;
    }

    Ok(())
}

pub fn selected_addons(
    specs: &[AddonSpec],
    selection: AddonSelection<'_>,
) -> Result<Vec<AddonSpec>> {
    match selection {
        AddonSelection::All => Ok(specs.to_vec()),
        AddonSelection::One(name) => {
            let selected = specs
                .iter()
                .copied()
                .filter(|spec| spec_matches(spec, name))
                .collect::<Vec<_>>();
            if selected.is_empty() {
                anyhow::bail!("unknown addon selection: {name}");
            }
            Ok(selected)
        }
    }
}

fn spec_matches(spec: &AddonSpec, name: &str) -> bool {
    [spec.label, spec.package_dir, spec.target_dir, spec.repo]
        .into_iter()
        .any(|value| value.eq_ignore_ascii_case(name))
}

fn update_addon(
    paths: &ProjectPaths,
    addon: AddonSpec,
    requested_ref: Option<&str>,
    dry_run: bool,
    user_agent: &str,
) -> Result<()> {
    let target = paths.godot_dir.join(addon.target_dir);
    assert_inside(&target, &paths.root)?;

    let current_version =
        read_plugin_version(&target.join("plugin.cfg")).unwrap_or_else(|_| "unknown".to_owned());

    let release = match requested_ref {
        Some(ref_name) => fetch_release_by_ref(addon, ref_name, user_agent)?,
        None => fetch_latest_release(addon, user_agent)?,
    };

    if requested_ref.is_none() && release_tag_matches_version(&release.tag, &current_version) {
        println!(
            "{}: already at {} ({})",
            addon.label, current_version, release.tag
        );
        return Ok(());
    }

    if dry_run {
        println!(
            "{}: would update {} -> {}",
            addon.label, current_version, release.tag
        );
        return Ok(());
    }

    let bytes = download_bytes(
        &release.download_url,
        matches!(addon.download, DownloadKind::ReleaseAssetZip),
        user_agent,
    )?;
    let temp = tempfile::tempdir().context("failed to create temp dir")?;
    extract_zip(&bytes, temp.path())?;

    let source = find_plugin_dir(temp.path(), addon.package_dir)
        .with_context(|| format!("{} package directory not found", addon.package_dir))?;
    let new_version = read_plugin_version(&source.join("plugin.cfg"))?;

    if release_files_match(&source, &target)? {
        println!(
            "{}: already matches {} ({})",
            addon.label, new_version, release.tag
        );
        return Ok(());
    }

    let uid_files = collect_uid_files(&target)?;
    replace_dir(&source, &target)?;
    restore_uid_files(&target, uid_files)?;

    println!(
        "{}: updated {} -> {} ({})",
        addon.label, current_version, new_version, release.tag
    );
    Ok(())
}

pub fn release_tag_matches_version(tag: &str, version: &str) -> bool {
    let tag = tag.trim_start_matches('v');
    tag == version
        || tag
            .strip_prefix(version)
            .is_some_and(|rest| matches!(rest.as_bytes().first(), Some(b'-' | b'+' | b'_')))
}

fn fetch_latest_release(addon: AddonSpec, user_agent: &str) -> Result<Release> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        addon.repo
    );
    let json: Value = ureq::get(&url)
        .set("User-Agent", user_agent)
        .call()
        .with_context(|| format!("GitHub API request failed: {url}"))?
        .into_json()
        .context("failed to parse GitHub release response")?;
    parse_release(addon, &json)
}

fn fetch_release_by_ref(addon: AddonSpec, ref_name: &str, user_agent: &str) -> Result<Release> {
    let release_url = format!(
        "https://api.github.com/repos/{}/releases/tags/{ref_name}",
        addon.repo
    );
    match ureq::get(&release_url).set("User-Agent", user_agent).call() {
        Ok(resp) => {
            let json: Value = resp
                .into_json()
                .context("failed to parse GitHub release response")?;
            parse_release(addon, &json)
        }
        Err(_) => Ok(Release {
            tag: ref_name.to_owned(),
            download_url: format!(
                "https://api.github.com/repos/{}/zipball/{ref_name}",
                addon.repo
            ),
        }),
    }
}

fn parse_release(addon: AddonSpec, json: &Value) -> Result<Release> {
    let tag = json["tag_name"]
        .as_str()
        .context("GitHub release response missing tag_name")?
        .to_owned();
    let zipball = json["zipball_url"]
        .as_str()
        .context("GitHub release response missing zipball_url")?;
    let download_url = match addon.download {
        DownloadKind::Zipball => zipball,
        DownloadKind::ReleaseAssetZip => json["assets"]
            .as_array()
            .and_then(|assets| {
                assets.iter().find_map(|asset| {
                    let name = asset["name"].as_str()?;
                    let url = asset["browser_download_url"].as_str()?;
                    name.ends_with(".zip").then_some(url)
                })
            })
            .unwrap_or(zipball),
    }
    .to_owned();
    Ok(Release { tag, download_url })
}

fn download_bytes(url: &str, accept_octet_stream: bool, user_agent: &str) -> Result<Vec<u8>> {
    match download_bytes_ureq(url, accept_octet_stream, user_agent) {
        Ok(bytes) => Ok(bytes),
        Err(first_error) => download_bytes_curl(url, accept_octet_stream, user_agent)
            .with_context(|| format!("ureq failed ({first_error:#}); curl fallback failed")),
    }
}

fn download_bytes_ureq(url: &str, accept_octet_stream: bool, user_agent: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut request = ureq::get(url).set("User-Agent", user_agent);
    if accept_octet_stream {
        request = request.set("Accept", "application/octet-stream");
    }
    request
        .call()
        .with_context(|| format!("download failed: {url}"))?
        .into_reader()
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read download: {url}"))?;
    Ok(bytes)
}

fn download_bytes_curl(url: &str, accept_octet_stream: bool, user_agent: &str) -> Result<Vec<u8>> {
    let file = tempfile::NamedTempFile::new().context("failed to create temp download file")?;
    let mut command = ProcessCommand::new("curl");
    command.args(["-L", "--fail", "--silent", "--show-error", "-A", user_agent]);
    if accept_octet_stream {
        command.args(["-H", "Accept: application/octet-stream"]);
    }
    let status = command
        .arg("-o")
        .arg(file.path())
        .arg(url)
        .status()
        .context("failed to start curl")?;
    if !status.success() {
        anyhow::bail!("curl exited with {status}");
    }
    fs::read(file.path()).context("failed to read curl download")
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<()> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).context("failed to open zip archive")?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let enclosed = file
            .enclosed_name()
            .context("zip archive contains an unsafe path")?
            .to_owned();
        let out = dest.join(enclosed);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::File::create(&out)?;
        io::copy(&mut file, &mut output)?;
    }
    Ok(())
}

fn find_plugin_dir(root: &Path, name: &str) -> Result<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dir.file_name().and_then(|part| part.to_str()) == Some(name)
            && dir.join("plugin.cfg").is_file()
        {
            return Ok(dir);
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    anyhow::bail!("plugin directory not found: {name}")
}

pub fn read_plugin_version(path: &Path) -> Result<String> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let version = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("version=\""))
        .and_then(|rest| rest.strip_suffix('"'))
        .context("plugin.cfg missing version")?;
    Ok(version.to_owned())
}

pub fn release_files_match(source: &Path, target: &Path) -> Result<bool> {
    if !target.is_dir() {
        return Ok(false);
    }

    let source_files = list_files(source)?;
    let source_set = source_files.iter().cloned().collect::<HashSet<_>>();
    for rel in list_files(target)? {
        if !source_set.contains(&rel) && rel.extension().and_then(|ext| ext.to_str()) != Some("uid")
        {
            return Ok(false);
        }
    }

    for rel in source_files {
        let left = fs::read(source.join(&rel))?;
        let right = match fs::read(target.join(&rel)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if left != right {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn collect_uid_files(target: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    if !target.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for rel in list_files(target)? {
        if rel.extension().and_then(|ext| ext.to_str()) == Some("uid") {
            files.push((rel.clone(), fs::read(target.join(rel))?));
        }
    }
    Ok(files)
}

pub fn restore_uid_files(target: &Path, files: Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
    for (rel, bytes) in files {
        let uid_path = target.join(&rel);
        if uid_path.exists() {
            continue;
        }
        let Some(stem) = uid_path.file_stem() else {
            continue;
        };
        if !uid_path.with_file_name(stem).exists() {
            continue;
        }
        if let Some(parent) = uid_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(uid_path, bytes)?;
    }
    Ok(())
}

pub fn replace_dir(source: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .with_context(|| format!("{} has no parent", target.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let file_name = target
        .file_name()
        .context("target directory has no file name")?
        .to_string_lossy();
    let temp_target = parent.join(format!(".{file_name}.upgrade-tmp"));
    if temp_target.exists() {
        fs::remove_dir_all(&temp_target)
            .with_context(|| format!("failed to remove {}", temp_target.display()))?;
    }
    copy_dir(source, &temp_target)?;
    if target.exists() {
        fs::remove_dir_all(target)
            .with_context(|| format!("failed to remove {}", target.display()))?;
    }
    fs::rename(&temp_target, target).with_context(|| {
        format!(
            "failed to move {} to {}",
            temp_target.display(),
            target.display()
        )
    })?;
    Ok(())
}

fn copy_dir(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn list_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    list_files_into(root, root, &mut files)?;
    Ok(files)
}

fn list_files_into(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            list_files_into(root, &path, files)?;
        } else {
            files.push(path.strip_prefix(root)?.to_path_buf());
        }
    }
    Ok(())
}

pub fn assert_inside(path: &Path, root: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let path = if path.exists() {
        path.canonicalize()?
    } else {
        canonicalize_existing_prefix(path)?
    };
    if path.starts_with(&root) {
        Ok(())
    } else {
        anyhow::bail!("refusing to edit outside repo: {}", path.display())
    }
}

fn canonicalize_existing_prefix(path: &Path) -> Result<PathBuf> {
    let mut cursor = path;
    let mut missing = Vec::<OsString>::new();
    while !cursor.exists() {
        let file_name = cursor
            .file_name()
            .with_context(|| format!("{} has no existing parent", path.display()))?;
        missing.push(file_name.to_os_string());
        cursor = cursor
            .parent()
            .with_context(|| format!("{} has no existing parent", path.display()))?;
    }
    let mut resolved = cursor.canonicalize()?;
    for part in missing.iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const LDTK: AddonSpec = AddonSpec {
        label: "LDtk Importer",
        repo: "heygleeson/godot-ldtk-importer",
        package_dir: "ldtk-importer",
        target_dir: "addons/ldtk-importer",
        download: DownloadKind::ReleaseAssetZip,
    };

    const ASEPRITE: AddonSpec = AddonSpec {
        label: "Aseprite Wizard",
        repo: "viniciusgerevini/godot-aseprite-wizard",
        package_dir: "AsepriteWizard",
        target_dir: "addons/AsepriteWizard",
        download: DownloadKind::Zipball,
    };

    #[test]
    fn release_tags_match_plugin_versions() {
        assert!(release_tag_matches_version("2.0.1", "2.0.1"));
        assert!(release_tag_matches_version("v2.0.1", "2.0.1"));
        assert!(release_tag_matches_version("v9.8.0-4", "9.8.0"));
        assert!(!release_tag_matches_version("2.0.1", "2.0"));
        assert!(!release_tag_matches_version("2.10.0", "2.1"));
    }

    #[test]
    fn selected_single_addon_is_exact() {
        let addons =
            selected_addons(&[LDTK, ASEPRITE], AddonSelection::One("AsepriteWizard")).unwrap();
        assert_eq!(addons, vec![ASEPRITE]);
    }

    #[test]
    fn reads_plugin_version() {
        let temp = TempDir::new().unwrap();
        let cfg = temp.path().join("plugin.cfg");
        fs::write(&cfg, "[plugin]\nversion=\"2.0.1\"\n").unwrap();
        assert_eq!(read_plugin_version(&cfg).unwrap(), "2.0.1");
    }

    #[test]
    fn replace_dir_copies_nested_files_and_removes_old_files() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(source.join("plugin.cfg"), "[plugin]\nversion=\"1\"\n").unwrap();
        fs::write(source.join("nested/file.gd"), "new").unwrap();
        fs::write(target.join("old.gd"), "old").unwrap();

        replace_dir(&source, &target).unwrap();

        assert_eq!(
            fs::read_to_string(target.join("nested/file.gd")).unwrap(),
            "new"
        );
        assert!(!target.join("old.gd").exists());
        assert_eq!(
            read_plugin_version(&target.join("plugin.cfg")).unwrap(),
            "1"
        );
    }

    #[test]
    fn release_files_match_ignores_preserved_uid_but_rejects_old_files() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(source.join("script.gd"), "extends Node").unwrap();
        fs::write(target.join("script.gd"), "extends Node").unwrap();
        fs::write(target.join("script.gd.uid"), "uid://abc").unwrap();

        assert!(release_files_match(&source, &target).unwrap());

        fs::write(target.join("old.gd"), "old").unwrap();
        assert!(!release_files_match(&source, &target).unwrap());
    }

    #[test]
    fn restore_uid_files_preserves_uid_when_script_exists() {
        let temp = TempDir::new().unwrap();
        let target = temp.path();
        fs::write(target.join("script.gd"), "extends Node").unwrap();
        let files = vec![(PathBuf::from("script.gd.uid"), b"uid://abc".to_vec())];
        restore_uid_files(target, files).unwrap();
        assert_eq!(
            fs::read(target.join("script.gd.uid")).unwrap(),
            b"uid://abc"
        );
    }

    #[test]
    fn restore_uid_files_skips_uid_when_script_missing() {
        let temp = TempDir::new().unwrap();
        let target = temp.path();
        let files = vec![(PathBuf::from("gone.gd.uid"), b"uid://abc".to_vec())];
        restore_uid_files(target, files).unwrap();
        assert!(!target.join("gone.gd.uid").exists());
    }

    #[test]
    fn assert_inside_rejects_parent_escape() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();

        assert!(assert_inside(&root.join("godot/addons/x"), &root).is_ok());
        assert!(assert_inside(&root.join("../outside"), &root).is_err());
    }
}
