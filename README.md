# cosmic-cassette-deck

A photoreal 1980s portable cassette player for your local music library. A native COSMIC desktop app built on libcosmic.

The deck is a photograph, and the app draws the living parts on top every frame: spinning reels with real spool physics, a rasterized track label, and transport keys that travel down into the chassis. Click a cassette case and it swings open, the tape lifts out, and it drops into the walkman. Recolor the deck and swap the room behind it.

<p align="center">
  <img src="docs/player-denim.png" alt="The photoreal cassette deck playing a track" width="760">
</p>

## Features

- Photoreal deck with animated reels driven by physically derived spool speeds. The source reel empties and speeds up while the take-up slows (an area-conserving pack model), and a fixed specular highlight is drawn over each spinning hub the way light stays put on a real deck.
- Case-open and insert animation. On the album screen, click the cassette case: it hinges open, the tape lifts out and drops into the deck, and playback begins.
- Dynamic label. Song and album come from your file tags, rasterized with ab_glyph and rotated (working around iced's lack of rotated canvas text), regenerated on track change rather than per frame.
- Colour skins: Denim Belt, Red, Blue, Black, and White, cycled from the Player.
- Swappable room backgrounds. Cycle the built-ins, add your own image, or remove ones you added. The choice applies to the rack, the album screen, and the insert animation.
- Album-cover wallpaper behind the Player deck. Toggle the Wallpaper button to fill the space around the deck with the playing album's cover art; off by default, and the choice is remembered.
- Cassette Rack: a fan-through-your-albums browser with cover art (from tags via lofty, or scraped), navigable with the arrow keys.
- Real player: local files, play, pause, stop, fast-forward and rewind (hold to wind), seek, volume, auto-advance across albums, and session resume.
- Mechanical sound. The transport keys click, the tape seats with a clunk when it drops into the deck, holding fast-forward or rewind whirrs for as long as it is held, and a tape reaching its end clunks to a stop before the next one loads. The cues are short FLAC clips embedded in the binary; if you build from source you can swap them for your own (see Assets).
- About panel showing the version and a link to the project, reachable from the header on the Player, the Cassette Rack, and the album screen.
- MPRIS support for media keys, playerctl, and desktop panel controls.
- Formats: MP3, FLAC, M4A/AAC, and WAV (rodio and symphonia).

## Screenshots

The Cassette Rack fans through your library, one cassette per album, with cover art from your tags (or scraped when a tag has none). Browse with the arrow keys or Prev and Next; Play loads the album at the front.

![Cassette Rack](docs/cassette-rack.png)

Click an album to bring its case forward, its cover printed on the J-card. Click the case, or Play, to open it.

![Album screen](docs/album.png)

The case hinges open, the tape lifts out, and it drops into the deck before playback begins.

![Insert animation](docs/insert.png)

Five colour skins cycle from the Player with the Walkman button: the same photographed deck, recolored. The player at the top of this page is the Denim Belt skin; here is Red, playing the same track.

![Red skin](docs/player-red.png)

## Install

There are two ways to get this app, and this README covers both. They are not the same job:

| | **Install** (this section) | **[Build from source](#building-from-source-for-hacking-on-it)** |
|---|---|---|
| You end up with | The app **installed** - system-wide, in your app menu as **Cassette Deck** | A **binary inside a checkout**, at `target/release/`. Nothing installed, nothing in your app menu |
| Effort | One command | Clone, install a Rust toolchain and system libraries, sit through a long first compile |
| Does it compile? | Ubuntu / Fedora / Flatpak: no, the binary is prebuilt. **NixOS: yes** - Nix compiles it for you, then installs it | Yes. You run `cargo` yourself |
| Dependencies | Pulled in for you | You install them yourself |
| Updating | Your package manager, or `nixos-rebuild` | `git pull` and recompile by hand |
| Removing it | `apt remove` / `dnf remove` / `flatpak uninstall` / drop it from your flake | Delete the checkout |
| Pick this if | You want to **use** the app | You want to **change the code**, or your distro has no package here |

**Almost everyone wants Install.** Note that on NixOS *both* routes compile the app - so "does it compile" is not the difference. The difference is whether you end up with an installed app or a checkout to work in.

Packages for the latest release are on the [Releases page](https://github.com/ctsdownloads/cosmic-cassette-deck/releases). Download the one for your distro, then install it from the directory you saved it in.

Ubuntu (24.04 and newer):

```sh
sudo apt install ./cosmic-cassette-deck_*_amd64.deb
```

Fedora (44 and newer):

```sh
sudo dnf install ./cosmic-cassette-deck-*.x86_64.rpm
```

Either one pulls in what the app needs at runtime - ALSA, Wayland, libxkbcommon, the Vulkan loader and driver, and the Adwaita icon theme - and adds **Cassette Deck** to your app menu.

Every prebuilt package here - `.deb`, `.rpm`, and the `.flatpak` below - is **x86_64 only**; they are all built on an x86_64 runner. There is no prebuilt ARM package. The Nix flake declares `aarch64-linux` and a source build has nothing x86-specific in it, so either should work on ARM by compiling on your machine - but neither is tested.

### Flatpak - any other distro, and the only sane route on atomic ones

Every release also carries a single-file `.flatpak` bundle. Download `cosmic-cassette-deck.flatpak` from the [Releases page](https://github.com/ctsdownloads/cosmic-cassette-deck/releases), then:

```sh
flatpak install --user ./cosmic-cassette-deck.flatpak
```

Use this on Arch, Debian, openSUSE, Mint, Pop!_OS, or anything else with no native package above.

**On atomic distros - Bluefin, Aurora, Bazzite, Silverblue, Kinoite - this is the route you want.** Layering the `.rpm` with `rpm-ostree` is an anti-pattern there, needs a reboot, and is disabled outright on some variants. The Flatpak is sandboxed, installs without touching the base image, and updates like any other app.

The bundle is sandboxed and ships its own icon theme, so the window controls render everywhere with no extra packages. It is not on Flathub, so it will not auto-update - grab the newer bundle from Releases and install it over the top.

**NixOS** - the `.deb` and `.rpm` are no use to you. [NIXOS_INSTALL.md](NIXOS_INSTALL.md) is your install route, and it covers three: `nix run` to try the app without installing it, `nix profile install` to install it with a single command, or adding it to your system flake so it is declarative and survives a rebuild. Nix compiles the app in all three cases; you never run cargo and you keep no checkout.

### Running outside COSMIC

It is a native COSMIC app, but it runs on any Wayland desktop - GNOME, KDE Plasma, and the rest. On those, it draws its titlebar's minimize, maximize, and close buttons from the Adwaita icon theme, because the COSMIC icon theme isn't installed there and the buttons would otherwise render blank. The packages depend on Adwaita, so an installed package just works. If you build from source, install it yourself; it is in the dependency lists below.

## Using it

### Add your music

Click **Open Folder** and point it at your music directory. The app scans it recursively, reads the tags, groups the files into album cassettes, and fills the rack. Click a cassette to play it.

Files are grouped into albums by their Artist and Album tags (read with lofty), so each album is one cassette no matter how many songs it holds, and its tracks play in track-number order. Files with no tags fall back to their `Artist - Title` filename, and each becomes its own single. Formats: MP3, FLAC, M4A/AAC, and WAV.

Your folder is remembered, so this is a one-time step. To point it somewhere else, click **Open Folder** again.

### Change the room background

The room behind the cassettes is swappable. On the Cassette Rack screen:

- **Backdrop** - cycle through the backgrounds, built-in and added.
- **Image** - add your own. PNG, JPG, or WEBP.
- **Remove** - remove the one currently showing, if it is one you added. The built-ins stay.

Your choice applies to the rack, the album screen, and the insert animation, and is remembered between runs.

### On the Player

- **Play** / **Pause**, plus stop, seek, and volume. Hold fast-forward or rewind to wind the tape.
- **Walkman** cycles the deck through its five colour skins.
- **Wallpaper: On / Off** fills the space around the deck with the playing album's cover art. Off by default, and remembered. This is separate from the room background above.
- **Music Rack** goes back to the shelf. **About** shows the version and a link to the project.

Playing a cassette starts at its first song and runs straight through the album, then rolls on to the next cassette on the shelf. To move between songs use **Prev** and **Next**, or slide the FUNCT switch on the deck so fast-forward and rewind step from track to track instead of winding the tape. The label always shows the song that is playing.

### Where your settings live

State is saved to `~/.config/cosmic-cassette-deck/state`: music folder, volume, current colour skin, chosen and added backgrounds, whether the Player wallpaper is on, and the last track and position for resume.

## Building from source (for hacking on it)

**This does not install the app.** You clone the repo and compile it; the binary lands in `target/release/` inside your checkout. It is not installed system-wide, it will not appear in your app menu, and nothing manages it for you. If you just want to *use* the app, go back to [Install](#install).

Use this if you want to change the code, or if your distro has no package above.

NixOS note: the [NIXOS_INSTALL.md](NIXOS_INSTALL.md) route compiles the app too - Nix just does it for you and installs the result. This section is different: it gives you a checkout to work in.

You need the Rust toolchain plus a few system libraries, then a single `cargo` build. There are two ways to get that environment: a normal distro with rustup, or NixOS using the dev shell this repo ships. Both finish at the same build step.

### 1. Clone the repo

```sh
git clone https://github.com/ctsdownloads/cosmic-cassette-deck.git
cd cosmic-cassette-deck
```

Run every command below from inside this `cosmic-cassette-deck` directory.

### 2. Get the toolchain and libraries

**On a normal distro** - install Rust with [rustup](https://rustup.rs), then the system libraries for your package manager:

Ubuntu (24.04 / 26.04):

```sh
sudo apt install build-essential pkg-config libasound2-dev libwayland-dev libxkbcommon-dev mesa-vulkan-drivers adwaita-icon-theme
```

Fedora (44):

```sh
sudo dnf install gcc pkg-config alsa-lib-devel wayland-devel libxkbcommon-devel vulkan-loader mesa-vulkan-drivers adwaita-icon-theme
```

Arch (rolling):

```sh
sudo pacman -S base-devel pkg-config alsa-lib wayland libxkbcommon vulkan-icd-loader adwaita-icon-theme
# plus your GPU's Vulkan driver: vulkan-radeon, vulkan-intel, or nvidia-utils
```

Package names vary, but in all cases you need a C toolchain, pkg-config, ALSA, Wayland, libxkbcommon, a Vulkan driver plus loader, and - on any desktop that is not COSMIC - the Adwaita icon theme, for the reason given above. The app runs under Wayland, and wgpu needs a Vulkan-capable GPU. File dialogs use the XDG desktop portal through rfd; COSMIC already ships xdg-desktop-portal-cosmic, other desktops need their own portal backend.

**On NixOS** - installing libraries system-wide isn't how NixOS works. Enter the dev shell this repo ships instead. It puts the Rust toolchain on your PATH and wires the build libraries above into the environment - `PKG_CONFIG_PATH` so cargo finds them, `LD_LIBRARY_PATH` so the binary runs - for the life of that shell only. Nothing is written into your system configuration:

```sh
nix develop        # flakes; or `nix-shell` on classic Nix
```

Run step 3 from inside it - `cargo build` outside the shell will fail, because none of those libraries are on the system. Leaving the shell (Ctrl-D) takes the toolchain back out again, but the compiled binary under `target/release/` stays put.

(To *install* the app on NixOS rather than compile it, you want [NIXOS_INSTALL.md](NIXOS_INSTALL.md), not this.)

### 3. Build and run

```sh
cargo run --release
```

The first build compiles libcosmic and takes several minutes. It produces the binary at `target/release/cosmic-cassette-deck` - run that directly next time, or `cargo run --release` again. (`Cargo.lock` is committed, so the build is reproducible; only regenerate it with `cargo generate-lockfile` if you change dependencies.)

On first run, see [Using it](#using-it) for adding your music and changing backgrounds.

## Tech

Rust, libcosmic (its vendored iced fork, wgpu renderer, Wayland and winit), rodio and symphonia (audio decode and playback), lofty (tags and embedded art), ab_glyph (label rasterization), mpris-server (MPRIS D-Bus), rfd (XDG-portal file picker), ureq with image and serde_json (album-art scraping), and tokio.

## Assets

Skin art lives in `assets/`, and the colour skins are recolored variants of the base deck.

Sound effects are FLAC clips in `assets/sfx/`: `key_press`, `tape_seat`, `wind_loop`, and `end_clack`. They are compiled into the binary, so swapping them means building from source: replace a clip with your own recording under the same name and rebuild. (An installed `.deb` or `.rpm` has them baked in - there is no file on disk to change.) Only `wind_loop` repeats; it is stitched seamless in code, so any length works.

## License

[GPL-3.0-only](LICENSE).
