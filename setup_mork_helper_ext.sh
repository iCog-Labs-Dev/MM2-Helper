#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
UPSTREAM_URL="${MORK_UPSTREAM_URL:-https://github.com/trueagi-io/MORK}"
PATHMAP_URL="https://github.com/Adam-Vandervorst/PathMap.git"
HELPER_EXT_SRC="$SCRIPT_DIR/helper_ext.rs"

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 /absolute/path/to/MORK" >&2
  exit 1
fi

# Normalize the requested MORK path so sibling paths are computed consistently.
MORK_INPUT="${1%/}"
mkdir -p "$(dirname "$MORK_INPUT")"
PARENT_DIR="$(cd "$(dirname "$MORK_INPUT")" && pwd)"
MORK_DIR="$PARENT_DIR/$(basename "$MORK_INPUT")"
PATHMAP_DIR="$PARENT_DIR/PathMap"

if [[ ! -f "$HELPER_EXT_SRC" ]]; then
  echo "ERROR: helper_ext.rs not found at $HELPER_EXT_SRC" >&2
  exit 1
fi

if [[ ! -d "$MORK_DIR/.git" ]]; then
  echo "MORK repo not found at $MORK_DIR. Cloning from $UPSTREAM_URL ..."
  git clone "$UPSTREAM_URL" "$MORK_DIR"
fi

# PathMap is expected to live beside the chosen MORK checkout.
if [[ ! -d "$PATHMAP_DIR/.git" ]]; then
  echo "PathMap repo not found at $PATHMAP_DIR. Cloning from $PATHMAP_URL ..."
  git clone "$PATHMAP_URL" "$PATHMAP_DIR"
fi

LIB_RS="$MORK_DIR/kernel/src/lib.rs"
SINKS_RS="$MORK_DIR/kernel/src/sinks.rs"
HELPER_EXT_DST="$MORK_DIR/kernel/src/helper_ext.rs"

for f in "$LIB_RS" "$SINKS_RS"; do
  if [[ ! -f "$f" ]]; then
    echo "ERROR: expected file missing: $f" >&2
    exit 1
  fi
done

echo "Copying helper extension file ..."
cp "$HELPER_EXT_SRC" "$HELPER_EXT_DST"

# Migrate legacy tuple_ext marker if present
perl -0pi -e 's/mod tuple_ext;/mod helper_ext;/' "$LIB_RS"
perl -0pi -e 's/tuple_ext::register\(&mut scope\);/helper_ext::register\(\&mut scope\);/g' "$SINKS_RS"
perl -0pi -e 's/use crate::tuple_ext;\n?//g' "$SINKS_RS"
perl -0pi -e 's/use crate::\{expr, pure, tuple_ext\};/use crate::{expr, pure, helper_ext};/' "$SINKS_RS"

# Ensure lib.rs has exactly one helper_ext module declaration, placed after mod pure;
perl -0pi -e 's/^mod helper_ext;\n//mg' "$LIB_RS"
if grep -q '^mod pure;$' "$LIB_RS"; then
  perl -0pi -e 's/mod pure;\n/mod pure;\nmod helper_ext;\n/' "$LIB_RS"
else
  echo "ERROR: expected 'mod pure;' in $LIB_RS" >&2
  exit 1
fi

# Ensure sinks.rs has exactly one helper_ext import
perl -0pi -e 's/^use crate::helper_ext;\n//mg' "$SINKS_RS"
if grep -q 'use crate::{expr, pure};' "$SINKS_RS"; then
  perl -0pi -e 's/use crate::\{expr, pure\};/use crate::{expr, pure};\nuse crate::helper_ext;/' "$SINKS_RS"
elif grep -q 'use crate::space::ACT_PATH;' "$SINKS_RS"; then
  perl -0pi -e 's/use crate::space::ACT_PATH;/use crate::helper_ext;\nuse crate::space::ACT_PATH;/' "$SINKS_RS"
else
  echo "ERROR: could not find a stable insertion point for helper_ext import in $SINKS_RS" >&2
  exit 1
fi

# Ensure helper_ext registration is present exactly once
perl -0pi -e 's/^\s*helper_ext::register\(&mut scope\);\n//mg' "$SINKS_RS"
if grep -q 'pure::register(&mut scope);' "$SINKS_RS"; then
  perl -0pi -e 's/pure::register\(&mut scope\);\n?/pure::register\(\&mut scope\);\n        helper_ext::register\(\&mut scope\);\n/' "$SINKS_RS"
else
  echo "ERROR: could not find pure::register(&mut scope); in $SINKS_RS" >&2
  exit 1
fi

echo "Building MORK ..."
(
  cd "$MORK_DIR/kernel"
  RUSTFLAGS="-C target-cpu=native" cargo build --release
)

# Cargo output location can vary depending on the workspace layout.
MORK_BIN="$MORK_DIR/kernel/target/release/mork"
if [[ ! -x "$MORK_BIN" ]]; then
  MORK_BIN="$MORK_DIR/target/release/mork"
fi

if [[ ! -x "$MORK_BIN" ]]; then
  echo "ERROR: could not locate the built mork binary." >&2
  exit 1
fi

MORK_BIN_DIR="$HOME/.local/bin"
mkdir -p "$MORK_BIN_DIR"
MORK_BIN_DIR="$(cd "$MORK_BIN_DIR" && pwd)"

# Refuse to overwrite an unrelated existing mork command.
if [[ -L "$MORK_BIN_DIR/mork" && "$(readlink "$MORK_BIN_DIR/mork")" != "$MORK_BIN" ]]; then
  echo "ERROR: $MORK_BIN_DIR/mork already exists and points somewhere else." >&2
  echo "Remove it and re-run." >&2
  exit 1
fi

if [[ -e "$MORK_BIN_DIR/mork" && ! -L "$MORK_BIN_DIR/mork" ]]; then
  echo "ERROR: $MORK_BIN_DIR/mork already exists and is not a symlink." >&2
  echo "Remove it and re-run." >&2
  exit 1
fi

rm -f "$MORK_BIN_DIR/mork"
ln -s "$MORK_BIN" "$MORK_BIN_DIR/mork"

# Make the user-level bin directory available in future shells if needed.
if [[ ":$PATH:" != *":$MORK_BIN_DIR:"* ]]; then
  PATH_LINE="export PATH=\"$MORK_BIN_DIR:\$PATH\""
  for rc in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.zshrc" "$HOME/.zprofile"; do
    touch "$rc"
    grep -Fqx "$PATH_LINE" "$rc" || printf '\n%s\n' "$PATH_LINE" >> "$rc"
  done
fi

echo
echo "Done."
echo "Binary: $MORK_BIN"
echo "Command: $MORK_BIN_DIR/mork"
if [[ ":$PATH:" != *":$MORK_BIN_DIR:"* ]]; then
  echo "Open a new shell before running mork."
else
  echo "You can now run mork from anywhere using: mork --help"
fi
