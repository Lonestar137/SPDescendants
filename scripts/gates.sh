#!/usr/bin/env bash
# The land gate chain: every gate in order, stopping at the first failure.
#
# fmt, clippy, data-guard, test, test-no-data. A failure prints the gate's log tail and exits
# nonzero; a green run stamps the tree, which then re-greens without running. GATES_FORCE=1 runs
# the chain anyway, TFD_DATA and MW2_DATA pick the data folders (set and empty: none) and TMPDIR
# holds the scratch files; the script sets SPDESC_REQUIRE_DATA / SPDESC_REQUIRE_MW2 and
# SPDESC_SKIP_LOG for its test rungs.
#
#   scripts/gates.sh
set -uo pipefail

# Gate the checkout $PWD is in, not this file's: a caller in one worktree may run another's copy
# by absolute path. Outside a checkout of this repo, gate this file's own checkout.
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
[ -n "$root" ] && [ -f "$root/scripts/gates.sh" ] || root="$here"
cd "$root" || exit 1
echo "gating: $root"

# ── Green-stamp memoization ──────────────────────────────────────────────────────────────────────
# A green run stamps target/.gates-green with the working tree's hash (untracked files included,
# through a throwaway index that leaves the real one alone), rustc's version, this script's hash
# and the data folder paths: the data-gated tests run only where a folder resolves, and the folders
# are gitignored, so the hash misses them. The data folders are compared last: resolving them is a
# `cargo run`.
stamp="target/.gates-green"
tree_key() {
    local tmpidx t
    tmpidx="$(mktemp "${TMPDIR:-/tmp}/spdesc-gates-idx.XXXXXX")" && rm -f "$tmpidx" || return 1
    t="$( (export GIT_INDEX_FILE="$tmpidx"
           git read-tree HEAD 2>/dev/null && git add -A . && git write-tree) 2>/dev/null )"
    rm -f "$tmpidx"
    [ -n "$t" ] || return 1
    printf '%s|%s|%s' "$t" "$(rustc -V 2>/dev/null)" \
        "$(shasum "$root/scripts/gates.sh" 2>/dev/null | cut -d' ' -f1)"
}
resolve_game_data() { cargo run -q -p spdesc-game-core --example where 2>/dev/null || true; }
resolve_mw2_data() { cargo run -q -p spdesc-game-core --example where mw2 2>/dev/null || true; }
# Both roots as one stamp field: a machine that gains or loses either folder must not reuse a
# verdict taken without it, and the data folders are gitignored, so the tree hash misses them.
data_roots() { printf '%s;%s' "$(resolve_game_data)" "$(resolve_mw2_data)"; }
# A delta only under docs/ or in top-level *.md files keeps the stamped verdict, since no gate
# reads those paths; a gate that starts to read one must narrow this pattern.
docs_only_delta() { # $1 = stamped tree, $2 = current tree
    git cat-file -e "$1^{tree}" 2>/dev/null && git cat-file -e "$2^{tree}" 2>/dev/null || return 1
    [ -z "$(git diff-tree -r --name-only "$1" "$2" 2>/dev/null | grep -vE '^(docs/|[^/]*\.md$)' || true)" ]
}
key_start="$(tree_key || true)"
if [ "${GATES_FORCE:-}" != "1" ] && [ -n "$key_start" ] && [ -f "$stamp" ]; then
    old="$(cat "$stamp" 2>/dev/null)"
    old_key="${old%|*}" # tree|rustc|script — the part that must match (bar the tree, see below)
    memo=""
    if [ "$old_key" = "$key_start" ]; then
        memo="this exact tree already passed"
    elif [ "${old_key#*|}" = "${key_start#*|}" ] && docs_only_delta "${old_key%%|*}" "${key_start%%|*}"; then
        memo="this tree differs from one that passed only under docs/ or in top-level *.md, which no gate reads"
    fi
    if [ -n "$memo" ] && [ "${old##*|}" = "$(data_roots)" ]; then
        echo "ALL GATES GREEN (memoized — $memo; GATES_FORCE=1 re-runs)"
        [ -d target ] && printf '%s\t%s\t%s\t%s\t%s\n' "$(date -u +%FT%TZ)" \
            "$(git rev-parse --short HEAD 2>/dev/null || echo '-')" chain 0 memo >>target/.gates-timing 2>/dev/null
        exit 0
    fi
fi

log="$(mktemp "${TMPDIR:-/tmp}/spdesc-gates.XXXXXX")"
skips="$(mktemp "${TMPDIR:-/tmp}/spdesc-gates-skips.XXXXXX")"
trap 'rm -f "$log" "$skips"' EXIT

# libtest hides a passing test's output, so `game_data`'s `skipped` also appends each data-gated
# skip to `$SPDESC_SKIP_LOG`; this prints them, grouped, after a test rung and empties the file.
report_skips() { # $1 = rung name
    [ -s "$skips" ] || return 0
    echo "  $1: $(wc -l <"$skips" | tr -d ' ') data-gated tests SKIPPED on this machine —"
    sort "$skips" | uniq -c | sort -rn | sed 's/^ *\([0-9]*\) \(.*\)/    \1 × \2/'
    echo "    (they run where the data is: docs/CONTRIBUTING.md, \"Setting up\")"
    : >"$skips"
}

# ── Timing ───────────────────────────────────────────────────────────────────────────────────────
# target/.gates-timing gets one tab-separated line per gate: UTC time, HEAD's short sha, gate,
# seconds, verdict. A memo hit and a skipped gate are logged at 0 s, the whole chain as `chain`.
timing="target/.gates-timing"
chain_t0=$SECONDS
tree_sha="$(git rev-parse --short HEAD 2>/dev/null || echo '-')"
note_timing() { # $1 = gate, $2 = seconds, $3 = verdict
    [ -d target ] || return 0
    printf '%s\t%s\t%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "$tree_sha" "$1" "$2" "$3" >>"$timing" 2>/dev/null || true
}

run() {
    local name="$1"
    shift
    local t0=$SECONDS
    if ! "$@" >"$log" 2>&1; then
        echo "GATE FAILED: $name ($((SECONDS - t0)) s)"
        note_timing "$name" "$((SECONDS - t0))" "FAILED"
        tail -30 "$log"
        exit 1
    fi
    echo "gate ok: $name ($((SECONDS - t0)) s)"
    note_timing "$name" "$((SECONDS - t0))" "ok"
}

run fmt cargo fmt --all -- --check
run clippy cargo clippy --workspace --all-targets -- -D warnings

# data-guard: nothing from either resolved data folder (nor the checkout's dev links) is tracked.
# Polishes commits, not installs: absent data passes.
run data-guard scripts/data-guard.sh

# test: where a data folder resolves, SPDESC_REQUIRE_DATA / SPDESC_REQUIRE_MW2 refuse a data-gated
# test that skips; without them a lost folder would read green. Each resolves once, for the stamp
# too. Each root has its own switch, so a machine with the game's data but no MW2 install still
# reports an honest skip from the sound tests rather than a failure.
game_data="$(resolve_game_data)"
mw2_data="$(resolve_mw2_data)"
roots="$(data_roots)"
require_data=""
require_mw2=""
if [ -n "$game_data" ]; then
    require_data=1
else
    echo "gates: NOTE — no TFD_DATA found here:" \
         "tests gated on the game's data skip silently on this machine"
fi
if [ -n "$mw2_data" ]; then
    require_mw2=1
else
    echo "gates: NOTE — no MW2_DATA found here:" \
         "tests gated on the sound data skip silently on this machine"
fi
run test env ${require_data:+SPDESC_REQUIRE_DATA=1} ${require_mw2:+SPDESC_REQUIRE_MW2=1} \
    SPDESC_SKIP_LOG="$skips" cargo test --workspace
report_skips test

# test-no-data: refuses a test that reads a data folder without a skip guard. `TFD_DATA=` and
# `MW2_DATA=` (set, empty) are the resolvers' "no data"; no build reads them, so the test binaries
# are reused.
run test-no-data env -u SPDESC_REQUIRE_DATA -u SPDESC_REQUIRE_MW2 TFD_DATA= MW2_DATA= \
    cargo test --workspace

echo "ALL GATES GREEN ($((SECONDS - chain_t0)) s; per-gate seconds in $timing)"
note_timing chain "$((SECONDS - chain_t0))" ok

# Stamp only if the tree still hashes as it did at the start: an edit during the run would
# otherwise memoize a green for a tree the chain never saw.
key_end="$(tree_key || true)"
if [ -n "$key_end" ] && [ "$key_end" = "$key_start" ] && [ -d target ]; then
    printf '%s|%s\n' "$key_end" "$roots" > "$stamp" 2>/dev/null || true
fi

# The clean-run gate needs a window: named here, not run.
echo "  (a live run of the app is a separate check: cargo play)"
