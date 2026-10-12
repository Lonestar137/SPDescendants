//! The game data folder's resolver finds, or every place it looked; exits 1 when there is none.
//! `scripts/gates.sh` and `scripts/data-guard.sh` ask this rather than repeating the rule.
//! `cargo run -p spdesc-game-core --example where`

fn main() -> std::process::ExitCode {
    match spdesc_game_core::game_data() {
        Some(data) => {
            println!("{}", data.display());
            std::process::ExitCode::SUCCESS
        }
        None => {
            eprintln!("no game data found. Looked in, in order:");
            for c in spdesc_game_core::candidates() {
                eprintln!("  {}", c.display());
            }
            eprintln!("Set $GAME_DATA, or link one at <checkout>/GAME_DATA.");
            std::process::ExitCode::FAILURE
        }
    }
}
