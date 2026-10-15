//! The data folder a resolver finds, or every place it looked; exits 1 when that one is not there.
//! `scripts/gates.sh` and `scripts/data-guard.sh` ask this rather than repeating the rule.
//! `cargo run -p spdesc-game-core --example where [tfd|mw2]` — no argument is `tfd`.

use std::process::ExitCode;

use spdesc_game_core::{DataRoot, MW2, TFD};

fn main() -> ExitCode {
    let root: DataRoot = match std::env::args().nth(1).as_deref() {
        None | Some("tfd") => TFD,
        Some("mw2") => MW2,
        Some(other) => {
            eprintln!("where: unknown data root `{other}` (want `tfd` or `mw2`)");
            return ExitCode::FAILURE;
        }
    };
    match root.resolve() {
        Some(data) => {
            println!("{}", data.display());
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("no {} folder found. Looked in, in order:", root.name);
            for c in root.candidates() {
                eprintln!("  {}", c.display());
            }
            eprintln!(
                "Set ${}, or link one at <checkout>/{}.",
                root.name, root.name
            );
            ExitCode::FAILURE
        }
    }
}
