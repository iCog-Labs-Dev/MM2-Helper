# MORK Helper Extension Package

Helper extension package for MORK grounding functions.

Current helpers included:

- `length`
- `car`
- `cdr`
- `cons`
- `decons`
- `partitions`



## What this package does

This package applies a small patch to a MORK checkout:

1. Copies `helper_ext.rs` into `kernel/src/helper_ext.rs`
2. Wires module/import/registration in kernel sources
3. Builds MORK release binary
4. Installs a user-level `mork` launcher so the binary can be run without an absolute path

## Requirements

- `git`
- Rust toolchain (nightly-compatible for MORK)
- `cargo`
- `perl`
- `rg` (ripgrep)

## Quick Start (from GitHub)

```bash
git clone https://github.com/tezena/MM2-Helper.git
cd MM2-Helper
bash setup_mork_helper_ext.sh /absolute/path/to/MORK
```

If `/absolute/path/to/MORK` does not exist, the script clones upstream MORK there automatically.
It also ensures a sibling `PathMap` checkout exists in the same parent directory as your chosen MORK path, cloning `https://github.com/Adam-Vandervorst/PathMap.git` when needed.

After the build, the script installs a `mork` launcher into a user-writable bin directory.
It uses `~/.local/bin`.
If needed, it appends that directory to `PATH` in your common bash and zsh startup files.
If `mork` already exists there and points somewhere else, the script stops instead of replacing it.

## Run MORK

```bash
mork run your_file.metta
```

## Re-run after updates

If you update this package later, run the setup command again:

```bash
bash setup_mork_helper_ext.sh /absolute/path/to/MORK
```

The script is designed to be repeatable and keeps wiring in sync.
