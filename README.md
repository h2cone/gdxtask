# gdxtask

Reusable Rust library for Godot + Rust (`gdext`) project `xtask` crates.

The crate keeps reusable workflow logic in a library while each game keeps a thin
project-owned binary with its own `clap` command enum.

## Features

- `process` (default): subprocess helpers.
- `cli`: reusable `clap` value enums and argument derives.
- `godot`: Godot cache and executable helpers.
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
