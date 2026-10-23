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
    r"^[ \t]*// MM2-HELPER-(?:FAST|FACTORIZED)-COUNT-BEGIN\n.*?^[ \t]*// MM2-HELPER-(?:FAST|FACTORIZED)-COUNT-END\n",
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

# Restore the original query expression before replacing the factorized wrapper.
# The wrapper contains upstream code and therefore cannot simply be deleted as
# the small dispatcher blocks can.
factorized_wrapper = re.compile(
    r"        // MM2-HELPER-FACTORIZED-COUNT-BEGIN\n"
    r".*?"
    r"            Self::query_multi\(&read_copy, pat_expr,(?P<body>.*?)\n"
    r"        \}\)\n"
    r"        \};\n"
    r"        // MM2-HELPER-FACTORIZED-COUNT-END\n",
    re.DOTALL,
)
space, wrapper_count = factorized_wrapper.subn(
    lambda match: (
        "        let touched = Self::query_multi(&read_copy, pat_expr,"
        + match.group("body")
        + "\n        });\n"
    ),
    space,
)
if "MM2-HELPER-FACTORIZED-COUNT-BEGIN" in space and wrapper_count != 1:
    raise SystemExit(f"ERROR: could not restore prior factorized-count wrapper in {space_path}")
space = generated_block.sub("", space)

# Remove the dispatcher entries installed by a previous run. The generated
# blocks above are marker-delimited; these enum and match entries cannot be.
sinks = sinks.replace("FastCountSink(helper_ext::FastCountSink), ", "")
sinks = sinks.replace("FactorizedCountSink(helper_ext::FactorizedCountSink), ", "")
sinks = sinks.replace(
    """        } else if unsafe { *e.ptr == item_byte(Tag::Arity(3)) && *e.ptr.offset(1) == item_byte(Tag::SymbolSize(10)) &&
            std::slice::from_raw_parts(e.ptr.offset(2), 10) == b\"count-fast\" } {
            ASink::FastCountSink(helper_ext::FastCountSink::new(e))
""",
    "",
)
sinks = sinks.replace(
    """        } else if unsafe { *e.ptr == item_byte(Tag::Arity(3)) && *e.ptr.offset(1) == item_byte(Tag::SymbolSize(16)) &&
            std::slice::from_raw_parts(e.ptr.offset(2), 16) == b\"count-factorized\" } {
            ASink::FactorizedCountSink(helper_ext::FactorizedCountSink::new(e))
""",
    "",
)
sinks = sinks.replace(
    "                ASink::FastCountSink(s) => { for i in s.request().into_iter() { yield i } }\n",
    "",
)
sinks = sinks.replace(
    "                ASink::FactorizedCountSink(s) => { for i in s.request().into_iter() { yield i } }\n",
    "",
)
sinks = sinks.replace(
    "            ASink::FastCountSink(s) => { s.sink(it, path) }\n",
    "",
)
sinks = sinks.replace(
    "            ASink::FactorizedCountSink(s) => { s.sink(it, path) }\n",
    "",
)
sinks = sinks.replace(
    "            ASink::FastCountSink(s) => { s.finalize(it) }\n",
    "",
)
sinks = sinks.replace(
    "            ASink::FactorizedCountSink(s) => { s.finalize(it) }\n",
    "",
)

sinks = sinks.replace(
    "CountSink(CountSink), HashSink(HashSink),",
    "CountSink(CountSink), FastCountSink(helper_ext::FastCountSink), FactorizedCountSink(helper_ext::FactorizedCountSink), HashSink(HashSink),",
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
            || matches!(self, ASink::FactorizedCountSink(_))
    }

    pub(crate) fn skip_rendered_match(&mut self) {
        match self {
            ASink::FastCountSink(s) => s.skip_rendered_match(),
            ASink::FactorizedCountSink(s) => s.skip_rendered_match(),
            _ => unreachable!("only counting sinks can skip template rendering"),
        }
    }
    // MM2-HELPER-FAST-COUNT-END

    // MM2-HELPER-FACTORIZED-COUNT-BEGIN
    pub(crate) fn is_factorized_count(&self) -> bool {
        matches!(self, ASink::FactorizedCountSink(_))
    }

    pub(crate) fn set_factorized_count(&mut self, count: usize) {
        if let ASink::FactorizedCountSink(s) = self {
            s.set_count(count);
        } else {
            unreachable!("only count-factorized accepts a grouped count");
        }
    }
    // MM2-HELPER-FACTORIZED-COUNT-END
"""
sinks = replace_once(sinks, asink_anchor, asink_extension, sinks_path)

count_dispatch = """            ASink::CountSink(CountSink::new(e))
        } else if unsafe {"""
fast_dispatch = """            ASink::CountSink(CountSink::new(e))
        } else if unsafe { *e.ptr == item_byte(Tag::Arity(3)) && *e.ptr.offset(1) == item_byte(Tag::SymbolSize(10)) &&
            std::slice::from_raw_parts(e.ptr.offset(2), 10) == b\"count-fast\" } {
            ASink::FastCountSink(helper_ext::FastCountSink::new(e))
        } else if unsafe { *e.ptr == item_byte(Tag::Arity(3)) && *e.ptr.offset(1) == item_byte(Tag::SymbolSize(16)) &&
            std::slice::from_raw_parts(e.ptr.offset(2), 16) == b"count-factorized" } {
            ASink::FactorizedCountSink(helper_ext::FactorizedCountSink::new(e))
        } else if unsafe {"""
sinks = replace_once(sinks, count_dispatch, fast_dispatch, sinks_path)

for old, new in (
    (
        "                ASink::CountSink(s) => { for i in s.request().into_iter() { yield i } }\n",
        "                ASink::CountSink(s) => { for i in s.request().into_iter() { yield i } }\n                ASink::FastCountSink(s) => { for i in s.request().into_iter() { yield i } }\n                ASink::FactorizedCountSink(s) => { for i in s.request().into_iter() { yield i } }\n",
    ),
    (
        "            ASink::CountSink(s) => { s.sink(it, path) }\n",
        "            ASink::CountSink(s) => { s.sink(it, path) }\n            ASink::FastCountSink(s) => { s.sink(it, path) }\n            ASink::FactorizedCountSink(s) => { s.sink(it, path) }\n",
    ),
    (
        "            ASink::CountSink(s) => { s.finalize(it) }\n",
        "            ASink::CountSink(s) => { s.finalize(it) }\n            ASink::FastCountSink(s) => { s.finalize(it) }\n            ASink::FactorizedCountSink(s) => { s.finalize(it) }\n",
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

function_start = space.index("    pub fn transform_multi_multi_o(")
query_start = space.index(
    "        let touched = Self::query_multi(&read_copy, pat_expr,",
    function_start,
)
query_end_marker = "        });\n\n        for (i, s) in sinks.iter_mut().enumerate() {"
query_end = space.index(query_end_marker, query_start)
query_prefix = """        // MM2-HELPER-FACTORIZED-COUNT-BEGIN
        let factorized_count = if sinks.len() == 1 && sinks[0].is_factorized_count() {
            crate::factorized_count::count(&read_copy, pat_expr)
        } else {
            None
        };
        let touched = if let Some(count) = factorized_count {
            sinks[0].set_factorized_count(count);
            count
        } else {
            Self::query_multi(&read_copy, pat_expr,"""
space = (
    space[:query_start]
    + query_prefix
    + space[query_start + len("        let touched = Self::query_multi(&read_copy, pat_expr,"):query_end]
    + "        })\n        };\n        // MM2-HELPER-FACTORIZED-COUNT-END\n\n        for (i, s) in sinks.iter_mut().enumerate() {"
    + space[query_end + len(query_end_marker):]
)

sinks_path.write_text(sinks)
space_path.write_text(space)
PY

echo "MM2-Helper fast count sink is wired into $MORK_DIR"
