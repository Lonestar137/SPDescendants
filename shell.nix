{ pkgs ? import <nixpkgs> { } }:

let
  runtimeLibraries = with pkgs; [
    alsa-lib
    fontconfig
    libx11
    libxcursor
    libxi
    libxinerama
    libxrandr
    libxkbcommon
    udev
    vulkan-loader
    wayland
  ];
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    rustup
    stdenv.cc
  ];

  buildInputs = runtimeLibraries;

  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibraries;

  shellHook = ''
    if [ -z "''${CARGO_TARGET_DIR:-}" ] && [ -d /mnt/ventoy/tmp/cache ]; then
      export CARGO_TARGET_DIR=/mnt/ventoy/tmp/cache
    fi
    # Player state must not ride $CARGO_TARGET_DIR (which may be a scratch disk): pin the state
    # folder to the tree so saves/settings persist independently of where the build lands.
    export SPDESC_HOME="$PWD/spdesc-config"
    # The game data folder (read-only, never committed). Default-expansion, no colon: an
    # explicitly empty `GAME_DATA=` (set, empty = "no data") must survive the hook, and only an
    # unset one falls back to the dev link beside the checkout.
    export GAME_DATA="''${GAME_DATA-$PWD/GAME_DATA}"
    printf 'Cargo target dir: %s\n' "''${CARGO_TARGET_DIR:-workspace target/}"
    printf 'Game data:      %s\n' "$GAME_DATA"
  '';
}
