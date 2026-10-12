//! Where the game data lives: the one resolver for the `GAME_DATA` folder. The first candidate
//! that exists wins:
//!
//! 1. `$GAME_DATA`. Set and empty (`GAME_DATA=`) means no data and ends the search, so the
//!    no-data path runs on a machine that has data.
//! 2. `<this checkout>/GAME_DATA`, the dev link, `dev` builds only ([`project_folder`]).
//! 3. `<exe dir>/GAME_DATA`, then `<exe dir>/data`, the shipped layouts.
//!
//! The folder is read-only runtime input: this module and everything that consumes it may read
//! it and must never write into it, and nothing under it is ever committed (see the project's
//! `docs/METHOD.md`).

use std::path::{Path, PathBuf};

/// Every place [`game_data`] looks, in order, whether or not it exists, for a "looked in" message.
pub fn candidates() -> Vec<PathBuf> {
    candidates_from(
        std::env::var_os("GAME_DATA").map(PathBuf::from),
        project_folder(),
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf)),
    )
}

/// [`candidates`] with the environment passed in: a test that set `$GAME_DATA` would change the
/// answer for every test running beside it. The `dev` fallback is passed in too, so the pure
/// search is testable without feature flags.
fn candidates_from(
    override_dir: Option<PathBuf>,
    project: Option<&Path>,
    exe_dir: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(4);

    // 1. The override; set and empty is no data at all, even in a dev build
    // (`GAME_DATA= cargo test` runs the no-data tests on a machine that has data).
    if let Some(over) = override_dir {
        if over.as_os_str().is_empty() {
            return Vec::new();
        }
        out.push(over);
    }

    // 2. The checkout's `GAME_DATA` link, dev builds only.
    if let Some(root) = project {
        out.push(root.join("GAME_DATA"));
    }

    // 3. Beside the binary; a failed `current_exe` adds nothing.
    if let Some(dir) = exe_dir {
        out.push(dir.join("GAME_DATA"));
        out.push(dir.join("data"));
    }

    out
}

/// This crate's manifest dir, two levels up: the checkout a `dev` build keeps its `GAME_DATA`
/// link in (gitignored; it points outside the checkout).
/// `None` in a build that turned the default `dev` feature off, which looks in no source tree.
#[cfg(feature = "dev")]
fn project_folder() -> Option<&'static Path> {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)
}

#[cfg(not(feature = "dev"))]
fn project_folder() -> Option<&'static Path> {
    None
}

/// The game data folder, or `None` when there is none. Uncached, so an environment change is
/// seen.
pub fn game_data() -> Option<PathBuf> {
    candidates().into_iter().find(|c| c.is_dir())
}

/// A data-gated test's skip: `game_data_or_skip!()` in a `-> ()` test,
/// `game_data_or_skip!(None)` in a helper returning `Option`. The skip names every path tried,
/// and under `SPDESC_REQUIRE_DATA=1` it fails instead.
#[macro_export]
macro_rules! game_data_or_skip {
    () => {
        $crate::game_data_or_skip!(())
    };
    ($ret:expr) => {
        match $crate::game_data() {
            Some(data) => data,
            None => {
                $crate::skipped("no game data found", &$crate::candidates());
                return $ret;
            }
        }
    };
}

/// A data-gated test's skip, printed; a panic under `SPDESC_REQUIRE_DATA=1`, which the gate sets
/// where the data is, since libtest hides a passing test's stderr and lost data reads green.
#[doc(hidden)]
pub fn skipped(what: &str, looked_in: &[PathBuf]) {
    skipped_under(
        std::env::var_os("SPDESC_REQUIRE_DATA").is_some_and(|v| !v.is_empty() && v != "0"),
        what,
        looked_in,
    );
    // `$SPDESC_SKIP_LOG`: libtest hides the line above, so the gate counts skips from this file.
    if let Some(log) = std::env::var_os("SPDESC_SKIP_LOG").filter(|p| !p.is_empty()) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
        {
            // One write per line: parallel tests append here, and a formatted write can interleave.
            let _ = f.write_all(format!("{what}\n").as_bytes());
        }
    }
}

/// [`skipped`] with the switch passed in, testable without the process environment.
fn skipped_under(required: bool, what: &str, looked_in: &[PathBuf]) {
    let msg = format!("skipping: {what} — looked in {looked_in:?}");
    assert!(
        !required,
        "{msg} — and SPDESC_REQUIRE_DATA is set: the gate resolved this data on this machine, \
         so a skip here is a broken resolver or a missing folder, not missing data"
    );
    eprintln!("{msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_empty_override_ends_the_search() {
        assert_eq!(
            candidates_from(Some(PathBuf::new()), Some(Path::new("/repo")), None),
            Vec::<PathBuf>::new(),
            "GAME_DATA= (set, empty) is 'no data', not a fallthrough"
        );
    }

    #[test]
    fn an_override_is_first_and_the_fallbacks_still_follow() {
        // A mistyped `GAME_DATA=` path falls through rather than dead-ending, so the search
        // stays one list the "looked in" message can print — the override is first, not alone.
        let got = candidates_from(
            Some(PathBuf::from("/mnt/data")),
            Some(Path::new("/repo")),
            None,
        );
        assert_eq!(
            got,
            vec![
                PathBuf::from("/mnt/data"),
                Path::new("/repo").join("GAME_DATA")
            ]
        );
    }

    #[test]
    fn without_an_override_the_checkout_then_the_binary_layout_are_tried_in_order() {
        let got = candidates_from(
            None,
            Some(Path::new("/repo")),
            Some(PathBuf::from("/opt/spdesc")),
        );
        assert_eq!(
            got,
            vec![
                Path::new("/repo").join("GAME_DATA"),
                Path::new("/opt/spdesc").join("GAME_DATA"),
                Path::new("/opt/spdesc").join("data"),
            ]
        );
    }

    #[test]
    fn no_candidates_at_all_is_a_skip_not_a_panic() {
        skipped_under(false, "no game data found", &[]);
    }

    #[test]
    #[should_panic(expected = "SPDESC_REQUIRE_DATA is set")]
    fn required_data_refuses_to_skip() {
        skipped_under(true, "no game data found", &[PathBuf::from("/nowhere")]);
    }

    #[test]
    fn the_dev_fallback_points_at_this_checkout() {
        #[cfg(feature = "dev")]
        assert_eq!(
            project_folder(),
            Some(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .ancestors()
                    .nth(2)
                    .unwrap()
            ),
            "the dev fallback is the checkout, two levels above this manifest"
        );
        #[cfg(not(feature = "dev"))]
        assert_eq!(
            project_folder(),
            None,
            "a shipped build looks in no source tree"
        );
    }
}
