# cosmic-cassette-deck

A photoreal 1980s portable cassette player for your local music library. A native COSMIC desktop app built on libcosmic.

The deck is a photograph, and the app draws the living parts on top every frame: spinning reels with real spool physics, a rasterized track label, and transport keys that travel down into the chassis. Click a cassette case and it swings open, the tape lifts out, and it drops into the walkman. Recolor the deck and swap the room behind it.

> Screenshots: capture the running app, drop them in `docs/`, and link them here.

## Features

- Photoreal deck with animated reels driven by physically derived spool speeds. The source reel empties and speeds up while the take-up slows (an area-conserving pack model), and a fixed specular highlight is drawn over each spinning hub the way light stays put on a real deck.
- Case-open and insert animation. On the album screen, click the cassette case: it hinges open, the tape lifts out and drops into the deck, and playback begins.
- Dynamic label. Song and album come from your file tags, rasterized with ab_glyph and rotated (working around iced's lack of rotated canvas text), regenerated on track change rather than per frame.
- Colour skins: silver, red, blue, black, and white, cycled from the Player.
- Swappable room backgrounds. Cycle the built-ins, add your own image, or remove ones you added. The choice applies to the rack, the album screen, and the insert animation.
- Cassette Rack: a fan-through-your-albums browser with cover art (from tags via lofty, or scraped), navigable with the arrow keys.
- Real player: local files, play, pause, stop, fast-forward and rewind (hold to wind), seek, volume, auto-advance across albums, and session resume.
- MPRIS support for media keys, playerctl, and desktop panel controls.
- Formats: MP3, FLAC, Ogg Vorbis, M4A/AAC, and WAV (rodio and symphonia).

## Building

Stable Rust plus a few system libraries. wgpu needs a Vulkan-capable GPU and driver, and the app runs under Wayland.

### Prerequisites

Install Rust with [rustup](https://rustup.rs), then the system development libraries.

Debian and Ubuntu:

```sh
sudo apt install build-essential pkg-config libasound2-dev libwayland-dev libxkbcommon-dev mesa-vulkan-drivers
```

Fedora:

```sh
sudo dnf install gcc pkg-config alsa-lib-devel wayland-devel libxkbcommon-devel vulkan-loader
```

Arch:

```sh
sudo pacman -S base-devel pkg-config alsa-lib wayland libxkbcommon vulkan-icd-loader
```

Package names vary by distribution. In all cases you need a C toolchain, pkg-config, ALSA, Wayland, libxkbcommon, and a Vulkan ICD. File dialogs use the XDG desktop portal through rfd. Under COSMIC, xdg-desktop-portal-cosmic is already present; on other desktops install the matching portal backend.

### Build and run

```sh
cargo run --release
```

This repo ships a committed `Cargo.lock`, so `cargo build` uses it as-is (if you change dependencies, regenerate it with `cargo generate-lockfile` and commit). Launch the app, open a music folder with the Open Folder button, and it scans the folder and fills the rack.

### NixOS

Use the bundled `shell.nix` (or the flake). It provides the toolchain and, just as importantly, sets `PKG_CONFIG_PATH`, `LD_LIBRARY_PATH`, and an rpath so the build links and the binary launches:

```sh
nix-shell
cargo build --release
```

`nix-shell` and `nix develop` are two entry points to the same dev environment; use whichever fits your setup:

- `nix-shell` reads `shell.nix` (the classic, non-flake file). Works on any Nix, no flags.
- `nix develop` reads the `flake.nix` devShell (the flakes system). Needs flakes enabled.

If you would rather put the toolchain in your system config, the packages are:

```nix
environment.systemPackages = with pkgs; [
  gcc cargo rustc rustfmt clippy pkg-config
  libxkbcommon wayland vulkan-loader libGL alsa-lib
];
```

Installing them alone is not enough on NixOS, though: you still have to set `PKG_CONFIG_PATH` (so pkg-config finds `alsa.pc` and `wayland.pc` at build time) and `LD_LIBRARY_PATH` (so the binary finds libwayland and vulkan at launch), which is exactly what `shell.nix` does for you. See `NIXOS_INSTALL.md` for installing the finished app as a system package.

## Usage and configuration

State is saved to `~/.config/cosmic-cassette-deck/state`: music folder, volume, current colour skin, chosen and added backgrounds, and the last track and position for resume.

- Player: transport controls, seek, and volume. The Walkman colour button cycles the deck colour.
- Cassette Rack: browse albums with the arrow keys. The backdrop controls cycle, add, and remove the room background. Click an album to open it.
- Album screen: click the case, or the play button, to open it and insert the tape.

## Tech

Rust, libcosmic (its vendored iced fork, wgpu renderer, Wayland and winit), rodio and symphonia (audio decode and playback), lofty (tags and embedded art), ab_glyph (label rasterization), mpris-server (MPRIS D-Bus), rfd (XDG-portal file picker), ureq with image and serde_json (album-art scraping), and tokio.

## Assets

Skin art lives in `assets/`, and the colour skins are recolored variants of the base deck. `tools/make_assets.py` is a legacy helper for regenerating base skin assets; its constants predate the current multi-skin layout.

## License

[GPL-3.0-only](LICENSE).
