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

## Run MORK

```bash
/absolute/path/to/MORK/target/release/mork run your_file.metta
```

## Re-run after updates

If you update this package later, run the setup command again:

```bash
bash setup_mork_helper_ext.sh /absolute/path/to/MORK
```

The script is designed to be repeatable and keeps wiring in sync.
