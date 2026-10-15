#!/usr/bin/env bash
# Re-copy the vendored IW4L crates from an IW4L checkout.
#
# `vendor/iw4/` holds third-party source on purpose — see `vendor/iw4/README.md` for why it is a
# copy rather than a submodule or a path dependency, and `NOTICE.md` for the licence. This script
# is the one-command way to refresh it: it copies the four crates, re-applies the two edits that
# make them stand outside their own workspace (explicit manifests, a provenance header), and
# records the revision it took them from.
#
# Running it twice on the same revision changes nothing, which is what makes a refresh a
# reviewable diff.
#
#   scripts/vendor-iw4.sh /path/to/iw4L [revision]
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
[ -n "$root" ] && [ -f "$root/scripts/vendor-iw4.sh" ] || root="$here"

upstream="${1:-}"
if [ -z "$upstream" ] || [ ! -d "$upstream/crates/fastfile_iw4" ]; then
    echo "usage: scripts/vendor-iw4.sh /path/to/iw4L [revision]" >&2
    echo "       (an IW4L checkout: <path>/crates/fastfile_iw4 must exist)" >&2
    exit 2
fi

rev="${2:-$(git -C "$upstream" rev-parse --short HEAD 2>/dev/null || echo unknown)}"
url="$(git -C "$upstream" config --get remote.origin.url 2>/dev/null || echo 'https://github.com/vladtrc/iw4L')"
url="${url%.git}"

# The four crates are the whole dependency closure of reading an IW4 zone: the container, the
# asset layouts it decodes into, and the two D3D9 value-semantics crates those layouts name.
crates="fastfile_iw4 asset_iw4 d3d9_decl d3d9_state"

for crate in $crates; do
    src="$upstream/crates/$crate"
    [ -d "$src/src" ] || { echo "vendor-iw4: $src/src is missing" >&2; exit 1; }
    dst="$root/vendor/iw4/$crate"
    rm -rf "$dst"
    mkdir -p "$dst"
    # `src/` only: the manifest is regenerated below, and upstream's tests are not ours to carry.
    cp -r "$src/src" "$dst/src"
done

# The manifests, spelled out rather than inherited: these crates live outside this workspace, so
# they cannot use `version.workspace = true` and must take their siblings by relative path.
cat >"$root/vendor/iw4/d3d9_decl/Cargo.toml" <<'EOF'
[package]
name = "d3d9_decl"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
publish = false
description = "D3D9 vertex declaration types and usage semantics. No IW4 knowledge."

[dependencies]

# Its own workspace root, so this repository's workspace selection, formatting and lint levels do
# not reach vendored third-party source. See `vendor/iw4/README.md`.
[workspace]
EOF

cat >"$root/vendor/iw4/d3d9_state/Cargo.toml" <<'EOF'
[package]
name = "d3d9_state"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
publish = false
description = "D3D9 render-state, blend, compare, cull, and sampler value semantics. No IW4 knowledge."

[dependencies]

# Its own workspace root, so this repository's workspace selection, formatting and lint levels do
# not reach vendored third-party source. See `vendor/iw4/README.md`.
[workspace]
EOF

cat >"$root/vendor/iw4/asset_iw4/Cargo.toml" <<'EOF'
[package]
name = "asset_iw4"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
publish = false
description = "Retail IW4 asset layouts — the IR every fastfile container decodes into."

[dependencies]
d3d9_decl = { path = "../d3d9_decl" }
d3d9_state = { path = "../d3d9_state" }

# Its own workspace root, so this repository's workspace selection, formatting and lint levels do
# not reach vendored third-party source. See `vendor/iw4/README.md`.
[workspace]
EOF

cat >"$root/vendor/iw4/fastfile_iw4/Cargo.toml" <<'EOF'
[package]
name = "fastfile_iw4"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
publish = false
description = "Retail IW4 zone container: header, stream, pointer fixup, asset table."

[dependencies]
asset_iw4 = { path = "../asset_iw4" }

# Its own workspace root, so this repository's workspace selection, formatting and lint levels do
# not reach vendored third-party source. See `vendor/iw4/README.md`.
[workspace]
EOF

# The full commit and its date, when the checkout can say: they go in NOTICE.md beside the rest.
full=""; date=""
if full="$(git -C "$upstream" rev-parse --verify --quiet "$rev^{commit}")"; then
    date="$(git -C "$upstream" log -1 --format=%cd --date=short "$rev")"
fi

# The provenance header on each crate root, the revision in `vendor/iw4/README.md`, and the
# revision in `NOTICE.md`.
python3 - "$root" "$rev" "$url" "$full" "$date" "$crates" <<'PY'
import pathlib
import sys

root, rev, url, full, date, crates = (
    pathlib.Path(sys.argv[1]),
    sys.argv[2],
    sys.argv[3],
    sys.argv[4],
    sys.argv[5],
    sys.argv[6].split(),
)

header = (
    f"// Vendored from IW4L ({url}) at commit {rev}, Apache-2.0.\n"
    "// See `vendor/iw4/README.md` for provenance and `NOTICE.md` at the repository root.\n"
    "// Unmodified upstream source apart from this notice and the crate's own manifest.\n"
    "\n"
)
for crate in crates:
    p = root / "vendor/iw4" / crate / "src/lib.rs"
    text = p.read_text()
    if text.startswith("// Vendored from IW4L"):
        text = text[text.index("\n\n") + 2 :]  # drop the notice already there
    p.write_text(header + text)

readme = root / "vendor/iw4/README.md"
lines = []
for line in readme.read_text().splitlines(keepends=True):
    if line.startswith("| Upstream |"):
        line = f"| Upstream | <{url}> |\n"
    elif line.startswith("| Commit |"):
        line = f"| Commit | `{full or rev}`{f' ({date})' if date else ''} |\n"
    lines.append(line)
readme.write_text("".join(lines))

notice = root / "NOTICE.md"
lines = []
for line in notice.read_text().splitlines(keepends=True):
    if line.startswith("Taken from upstream commit"):
        # The short revision here, the full commit in `vendor/iw4/README.md`: one readable
        # reference beside the text, one exact one in the provenance table.
        line = f"Taken from upstream commit `{rev}`"
        line += f" ({date})." if date else "."
        line += " The source is unmodified apart from a provenance\n"
    lines.append(line)
notice.write_text("".join(lines))
PY

echo "vendor-iw4: copied $crates at $rev"
git -C "$root" --no-pager diff --stat -- vendor/ | tail -1 || true
echo "vendor-iw4: review with 'git diff -- vendor/'"
