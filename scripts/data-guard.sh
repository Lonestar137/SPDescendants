#!/usr/bin/env bash
# Refuses a tracked entry inside a resolved data folder and the checkout's dev links: those folders
# are read-only runtime input and nothing from them — not even the links — is ever committed
# (docs/METHOD.md). `TFD_DATA` supplies the game's own assets, `MW2_DATA` the sound effects; both
# are guarded the same way. A folder outside this checkout cannot be tracked by this repo, so only
# in-tree resolutions are checked. `.gitignore` covers the links; this catches a force-add and any
# in-tree copy.
#
#   scripts/data-guard.sh
set -uo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
[ -n "$root" ] && [ -f "$root/scripts/data-guard.sh" ] || root="$here"
cd "$root" || exit 1

fail=0

# Fail when any tracked file sits under $1, given as a path inside this checkout.
guard() { # $1 = absolute folder, $2 = how it was named
    local dir="$1" label="$2" rel n
    case "$dir" in
    "$root"/*) ;;
    *) return 0 ;; # outside the checkout: git cannot track it from here
    esac
    rel="${dir#"$root"/}"
    [ -n "$rel" ] || { echo "data-guard: $label resolves to the checkout root — refusing"; fail=1; return; }
    n="$(git ls-files -- "$rel" | wc -l)"
    if [ "$n" -ne 0 ]; then
        echo "data-guard: FAILED — $n tracked file(s) under $label ($dir):"
        git ls-files -- "$rel" | sed 's/^/  /'
        fail=1
    fi
}

# Both resolved data folders, asked of the workspace's one resolver (never a second copy of the
# rule). Absent data is not a failure: this guard polishes commits, not installs.
for name in tfd mw2; do
    data="$(cargo run -q -p spdesc-game-core --example where "$name" 2>/dev/null || true)"
    if [ -n "$data" ]; then
        guard "$data" "${name^^}_DATA"
    fi
done

# The dev links themselves, whether or not they resolve today: they are machine-local by definition.
guard "$root/TFD_DATA" 'TFD_DATA (the dev link)'
guard "$root/MW2_DATA" 'MW2_DATA (the dev link)'

if [ "$fail" -ne 0 ]; then
    echo "data-guard: a data folder is read-only runtime input — untrack those files, keep the"
    echo "data-guard: folder outside the checkout or gitignored (docs/METHOD.md, \"Hard rules\")"
    exit 1
fi
echo "data-guard ok: nothing from a data folder is tracked"
