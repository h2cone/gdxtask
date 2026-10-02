# gdxtask

Reusable Rust library for Godot + Rust (`gdext`) project `xtask` crates.

The crate keeps reusable workflow logic in a library while each game keeps a thin
project-owned binary with its own `clap` command enum.

## Features

- `process` (default): subprocess helpers.
- `cli`: reusable `clap` value enums and argument derives.
- `godot`: Godot cache/executable helpers and strict headless script validation.
- `files`: atomic file replacement with bounded Windows sharing-conflict retries.
- `isolation`: temporary projects and generated-output consistency checks.
- `gdext`: update a git-pinned `dependencies.godot.rev`.
- `addons`: update Godot addons from GitHub release assets or zipballs.
- `run-export`: reusable `run` and `export` commands.

## Thin xtask Example

```toml
[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
gdxtask = { path = "../../gdxtask", features = ["cli", "addons", "gdext", "run-export"] }
```

See [`examples/full.rs`](examples/full.rs) for a complete project-side wrapper.

## Standard Layout

By default `ProjectPaths` expects:

```text
repo/
  rust/Cargo.toml
  godot/project.godot
  export/
```

Use `paths::Layout` and `paths::Validate` when a project needs custom directory
names or additional validation such as `godot/addons`.

## Headless validation and executable discovery

`godot::find_godot(None)` searches `GODOT4_BIN`, `GODOT_BIN`, then
`godot4`/`godot` on PATH. Pass `Some(path)` to select an explicit path or
command name. Invalid explicit/environment configuration is an error; Windows
still prefers an existing `_console` sibling. `RunArgs` and `ExportArgs` use an
empty executable string for this search by default; nonempty values are explicit.

Headless tasks should validate captured output as well as the exit code:

```rust,no_run
use gdxtask::godot::{OutputPolicy, ScriptOptions, find_godot, run_script};
use std::path::Path;

let executable = find_godot(None)?;
let options = ScriptOptions {
    output: OutputPolicy {
        success_marker: Some("CONTENT_VALID"),
        error_prefixes: &["SCRIPT ERROR", "ERROR:", "CONTENT_ERROR:"],
        ..Default::default()
    },
    ..Default::default()
};
run_script(Path::new("godot"), &executable, "res://validate.gd", "content validation", &options)?;
# Ok::<(), anyhow::Error>(())
```

`OutputPolicy` rejects Godot errors on either stream even after a successful
exit. Markers must appear on stdout. Known shutdown leak diagnostics are only
allowed when the caller sets `allow_shutdown_leaks`; other errors still fail.
`script_command` exposes the same command construction for custom logging.
`quit_after` is an optional frame limit for synchronous scripts, not a process
timeout; asynchronous scripts should leave it unset.

## File publication (`files` feature)

`files::atomic_write(path, bytes, &ReplaceOptions::default())` stages a uniquely
named file beside its destination, syncs it, closes it, and replaces the existing
file. Failed staging is cleaned up. On Windows, sharing/access conflicts are
retried with bounded exponential backoff; timeout and delays are configurable.
A zero timeout disables retries. Other platforms use rename without retries.

`files::write_synced` and `files::replace_file` support caller-owned staging.
Atomicity is per file: these helpers do not provide a multi-file transaction or
guarantee directory durability after a crash.

## Isolated generated-output checks (`isolation` feature)

`isolation::IsolatedProject::new` copies the Godot project without `.godot` into
a temporary directory, respects the supplied `Layout`, and copies explicitly
selected dependencies through `ProjectFile` destinations relative to the scratch
repository. It removes the scratch tree when dropped. Symlinks are rejected.

Build an `ArtifactSnapshot` containing only project-owned generated outputs.
Pass it and a regeneration callback to `isolation::check_generation`: the callback
runs twice, its results must be byte-identical, and the committed outputs must
match the first result. The callback owns any project-specific validation.
`differences` and `ensure_matches` report missing, unexpected, and changed files
in stable path order, including the first differing byte for changed files.

The library imposes no LDtk entity schema, room catalog, Aseprite manifest,
GDExtension filename, or generated artifact naming convention.
