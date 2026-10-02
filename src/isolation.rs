//! Temporary Godot projects and deterministic generated-output checks.
//! Generation rules and the selected artifact set remain project-owned.

use crate::paths::{Layout, ProjectPaths};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
};
use tempfile::TempDir;

/// Copy a selected dependency to a repository-relative path in the scratch tree.
#[derive(Clone, Copy, Debug)]
pub struct ProjectFile<'a> {
    pub source: &'a Path,
    pub destination: &'a Path,
}

/// Owns the scratch directory for as long as commands use its project paths.
#[derive(Debug)]
pub struct IsolatedProject {
    directory: TempDir,
    pub paths: ProjectPaths,
}

impl IsolatedProject {
    pub fn new(
        source: &ProjectPaths,
        layout: &Layout,
        extra_files: &[ProjectFile<'_>],
    ) -> Result<Self> {
        for subdir in [
            &layout.godot_subdir,
            &layout.rust_subdir,
            &layout.export_subdir,
        ] {
            check_relative(subdir)?;
        }
        let directory = TempDir::new().context("create isolated Godot project")?;
        let paths = ProjectPaths::from_root_unchecked(directory.path().to_path_buf(), layout);
        copy_tree(&source.godot_dir, &paths.godot_dir, &[".godot"])?;
        for file in extra_files {
            check_relative(file.destination)?;
            let destination = paths.root.join(file.destination);
            fs::create_dir_all(
                destination
                    .parent()
                    .context("isolated dependency has no parent")?,
            )?;
            fs::copy(file.source, &destination).with_context(|| {
                format!(
                    "copy isolated dependency {} -> {}",
                    file.source.display(),
                    destination.display()
                )
            })?;
        }
        Ok(Self { directory, paths })
    }

    pub fn root(&self) -> &Path {
        self.directory.path()
    }
}

fn check_relative(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "isolated destination must be a nonempty relative path without traversal: {}",
        path.display()
    );
    Ok(())
}

/// Copy a tree, excluding directory/file names at every depth. Symlinks are
/// rejected rather than followed into an unrelated tree.
pub fn copy_tree(source: &Path, destination: &Path, excluded_names: &[&str]) -> Result<()> {
    ensure!(
        !fs::symlink_metadata(source)?.file_type().is_symlink(),
        "cannot isolate symlink {}",
        source.display()
    );
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry?;
        if excluded_names.iter().any(|name| entry.file_name() == *name) {
            continue;
        }
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "cannot isolate symlink {}",
            from.display()
        );
        if kind.is_dir() {
            copy_tree(&from, &to, excluded_names)?;
        } else {
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} -> {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

pub type ArtifactSnapshot = BTreeMap<String, Vec<u8>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DifferenceKind {
    Missing,
    Unexpected,
    Changed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDifference {
    pub path: String,
    pub kind: DifferenceKind,
    pub first_differing_byte: Option<usize>,
    pub expected_len: Option<usize>,
    pub actual_len: Option<usize>,
}

/// Include added/removed paths as well as byte differences, in stable path order.
pub fn differences(
    expected: &ArtifactSnapshot,
    actual: &ArtifactSnapshot,
) -> Vec<ArtifactDifference> {
    let paths: BTreeSet<_> = expected.keys().chain(actual.keys()).collect();
    paths
        .into_iter()
        .filter_map(|path| {
            let before = expected.get(path);
            let after = actual.get(path);
            if before == after {
                return None;
            }
            let kind = match (before, after) {
                (Some(_), None) => DifferenceKind::Missing,
                (None, Some(_)) => DifferenceKind::Unexpected,
                _ => DifferenceKind::Changed,
            };
            let first_differing_byte = before.zip(after).and_then(|(before, after)| {
                (0..before.len().max(after.len()))
                    .find(|index| before.get(*index) != after.get(*index))
            });
            Some(ArtifactDifference {
                path: path.clone(),
                kind,
                first_differing_byte,
                expected_len: before.map(Vec::len),
                actual_len: after.map(Vec::len),
            })
        })
        .collect()
}

pub fn ensure_matches(
    expected: &ArtifactSnapshot,
    actual: &ArtifactSnapshot,
    label: &str,
) -> Result<()> {
    let changed = differences(expected, actual);
    ensure!(
        changed.is_empty(),
        "{label}: {} differing artifacts; first differences: {:?}",
        changed.len(),
        &changed[..changed.len().min(8)]
    );
    Ok(())
}

/// Run a project-supplied generator twice in its prepared scratch environment,
/// checking byte idempotence and committed output freshness. The caller chooses
/// validation steps and snapshots; no LDtk/Aseprite schema is imposed here.
pub fn check_generation(
    committed: &ArtifactSnapshot,
    mut regenerate: impl FnMut() -> Result<ArtifactSnapshot>,
) -> Result<ArtifactSnapshot> {
    let first = regenerate().context("first isolated generation")?;
    let second = regenerate().context("second isolated generation")?;
    ensure_matches(&first, &second, "generation is not byte-idempotent")?;
    ensure_matches(
        &first,
        committed,
        "committed outputs differ from isolated generation",
    )?;
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_projects_support_custom_layout_and_explicit_dependencies() {
        let source = TempDir::new().unwrap();
        let layout = Layout {
            rust_subdir: "native".into(),
            godot_subdir: "game".into(),
            export_subdir: "dist".into(),
        };
        let paths = ProjectPaths::from_root_unchecked(source.path().into(), &layout);
        fs::create_dir_all(paths.godot_dir.join(".godot/imported")).unwrap();
        fs::write(paths.godot_dir.join("project.godot"), "source").unwrap();
        fs::write(paths.godot_dir.join(".godot/imported/cached"), "cache").unwrap();
        let library = source.path().join("library");
        fs::write(&library, "extension").unwrap();
        let scratch = IsolatedProject::new(
            &paths,
            &layout,
            &[ProjectFile {
                source: &library,
                destination: Path::new("native/build/custom.dylib"),
            }],
        )
        .unwrap();
        assert_eq!(
            fs::read(scratch.paths.godot_dir.join("project.godot")).unwrap(),
            b"source"
        );
        assert!(!scratch.paths.godot_dir.join(".godot").exists());
        assert_eq!(
            fs::read(scratch.paths.rust_dir.join("build/custom.dylib")).unwrap(),
            b"extension"
        );
        let scratch_root = scratch.root().to_path_buf();
        drop(scratch);
        assert!(!scratch_root.exists());
        assert!(paths.godot_dir.join(".godot/imported/cached").exists());
        assert!(
            IsolatedProject::new(
                &paths,
                &layout,
                &[ProjectFile {
                    source: &library,
                    destination: Path::new("../escaped")
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn generation_checks_detect_drift_non_idempotence_and_missing_outputs() {
        let committed = ArtifactSnapshot::from([("a".into(), b"abc".to_vec())]);
        let mut calls = 0;
        assert_eq!(
            check_generation(&committed, || {
                calls += 1;
                Ok(committed.clone())
            })
            .unwrap(),
            committed
        );
        assert_eq!(calls, 2);
        let error = check_generation(&ArtifactSnapshot::new(), || Ok(committed.clone()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Missing") && error.contains("a"));
        let mut pass = 0;
        let error = check_generation(&committed, || {
            pass += 1;
            Ok(ArtifactSnapshot::from([("a".into(), vec![pass])]))
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("not byte-idempotent"));
        let changed = differences(
            &committed,
            &ArtifactSnapshot::from([("a".into(), b"abcd".to_vec()), ("b".into(), vec![])]),
        );
        assert_eq!(changed[0].first_differing_byte, Some(3));
        assert_eq!(changed[1].kind, DifferenceKind::Unexpected);
    }

    #[cfg(unix)]
    #[test]
    fn isolation_rejects_symlinks_instead_of_following_them() {
        let source = TempDir::new().unwrap();
        let destination = TempDir::new().unwrap();
        std::os::unix::fs::symlink(destination.path(), source.path().join("link")).unwrap();
        assert!(copy_tree(source.path(), &destination.path().join("copy"), &[]).is_err());
    }
}
