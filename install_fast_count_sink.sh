#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 1 ]]; then
  echo "Usage: $0 /path/to/MORK" >&2
  exit 1
fi

MORK_DIR="$(cd "$1" && pwd)"
SINKS_RS="$MORK_DIR/kernel/src/sinks.rs"
SPACE_RS="$MORK_DIR/kernel/src/space.rs"

for file in "$SINKS_RS" "$SPACE_RS"; do
  if [[ ! -f "$file" ]]; then
    echo "ERROR: expected file missing: $file" >&2
    exit 1
  fi
done

python3 - "$SINKS_RS" "$SPACE_RS" <<'PY'
from pathlib import Path
import re
import sys

sinks_path = Path(sys.argv[1])
space_path = Path(sys.argv[2])

generated_block = re.compile(
    r"^[ \t]*// MM2-HELPER-FAST-COUNT-BEGIN\n.*?^[ \t]*// MM2-HELPER-FAST-COUNT-END\n",
    re.MULTILINE | re.DOTALL,
)

def replace_once(text, old, new, path):
    if old not in text:
        raise SystemExit(f"ERROR: could not find fast-count insertion point in {path}")
    return text.replace(old, new, 1)

sinks = sinks_path.read_text()
space = space_path.read_text()

# The installer is repeatable. Remove its prior generated blocks before
# applying the current helper-owned implementation.
sinks = generated_block.sub("", sinks)
space = generated_block.sub("", space)

# Remove the dispatcher entries installed by a previous run. The generated
# blocks above are marker-delimited; these enum and match entries cannot be.
sinks = sinks.replace("FastCountSink(helper_ext::FastCountSink), ", "")
sinks = sinks.replace(
    """        } else if unsafe { *e.ptr == item_byte(Tag::Arity(3)) && *e.ptr.offset(1) == item_byte(Tag::SymbolSize(10)) &&
            std::slice::from_raw_parts(e.ptr.offset(2), 10) == b\"count-fast\" } {
            ASink::FastCountSink(helper_ext::FastCountSink::new(e))
""",
    "",
)
sinks = sinks.replace(
    "                ASink::FastCountSink(s) => { for i in s.request().into_iter() { yield i } }\n",
    "",
)
sinks = sinks.replace(
    "            ASink::FastCountSink(s) => { s.sink(it, path) }\n",
    "",
)
sinks = sinks.replace(
    "            ASink::FastCountSink(s) => { s.finalize(it) }\n",
    "",
)

sinks = sinks.replace(
    "CountSink(CountSink), HashSink(HashSink),",
    "CountSink(CountSink), FastCountSink(helper_ext::FastCountSink), HashSink(HashSink),",
    1,
)

asink_anchor = """    pub fn compat(e: Expr) -> Self {
        ASink::CompatSink(CompatSink::new(e))
    }
"""
asink_extension = asink_anchor + """
    // MM2-HELPER-FAST-COUNT-BEGIN
    pub(crate) fn can_skip_render(&self) -> bool {
        matches!(self, ASink::FastCountSink(s) if s.can_skip_render())
    }

    pub(crate) fn skip_rendered_match(&mut self) {
        if let ASink::FastCountSink(s) = self {
            s.skip_rendered_match();
        } else {
            unreachable!("only count-fast can skip template rendering");
        }
    }
    // MM2-HELPER-FAST-COUNT-END
"""
sinks = replace_once(sinks, asink_anchor, asink_extension, sinks_path)

count_dispatch = """            ASink::CountSink(CountSink::new(e))
        } else if unsafe {"""
fast_dispatch = """            ASink::CountSink(CountSink::new(e))
        } else if unsafe { *e.ptr == item_byte(Tag::Arity(3)) && *e.ptr.offset(1) == item_byte(Tag::SymbolSize(10)) &&
            std::slice::from_raw_parts(e.ptr.offset(2), 10) == b\"count-fast\" } {
            ASink::FastCountSink(helper_ext::FastCountSink::new(e))
        } else if unsafe {"""
sinks = replace_once(sinks, count_dispatch, fast_dispatch, sinks_path)

for old, new in (
    (
        "                ASink::CountSink(s) => { for i in s.request().into_iter() { yield i } }\n",
        "                ASink::CountSink(s) => { for i in s.request().into_iter() { yield i } }\n                ASink::FastCountSink(s) => { for i in s.request().into_iter() { yield i } }\n",
    ),
    (
        "            ASink::CountSink(s) => { s.sink(it, path) }\n",
        "            ASink::CountSink(s) => { s.sink(it, path) }\n            ASink::FastCountSink(s) => { s.sink(it, path) }\n",
    ),
    (
        "            ASink::CountSink(s) => { s.finalize(it) }\n",
        "            ASink::CountSink(s) => { s.finalize(it) }\n            ASink::FastCountSink(s) => { s.finalize(it) }\n",
    ),
):
    sinks = replace_once(sinks, old, new, sinks_path)

write_pattern = re.compile(
    r"(?P<head>                    'writes : for \(i, template\) in templates\.iter\(\)\.enumerate\(\) \{\n)"
    r"(?:[ \t]*\n)*(?P<indent>                        )(?P<write>let wz = unsafe \{ std::ptr::read\(&template_resources\[subsumption\[i\]\]\) \};\n)"
)

def install_write_skip(match):
    return (
        match.group("head")
        + "                        // MM2-HELPER-FAST-COUNT-BEGIN\n"
        + "                        if sinks[i].can_skip_render() {\n"
        + "                            sinks[i].skip_rendered_match();\n"
        + "                            continue 'writes;\n"
        + "                        }\n"
        + "                        // MM2-HELPER-FAST-COUNT-END\n"
        + match.group("indent")
        + match.group("write")
    )

space, write_count = write_pattern.subn(install_write_skip, space)
if write_count != 2:
    raise SystemExit(f"ERROR: expected two fast-count write loops in {space_path}, found {write_count}")

sinks_path.write_text(sinks)
space_path.write_text(space)
PY

echo "MM2-Helper fast count sink is wired into $MORK_DIR"
