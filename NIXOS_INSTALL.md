# Cassette Deck - NixOS install (panel applet the Nix way)

This packages the player + panel applet exactly like your other COSMIC applets
(cosmic-app-volume, cosmic-camera-controls, etc). The cassette icon appears in
the COSMIC panel and launches the full player.

## 1. Place the package in your config

Copy this whole directory to:

    ~/nixos-config/packages/cosmic-cassette-deck/

## 2. Generate Cargo.lock (required by the Nix build)

The Nix build needs a committed Cargo.lock. Generate it once:

    cd ~/nixos-config/packages/cosmic-cassette-deck
    nix develop -c cargo generate-lockfile
    # (or: nix-shell -p cargo --run "cargo generate-lockfile")

## 3. Add it as a flake input

In `~/nixos-config/flake.nix`, alongside your other cosmic-* inputs:

    cosmic-cassette-deck = {
      url = "path:./packages/cosmic-cassette-deck";
      inputs.nixpkgs.follows = "nixpkgs";
    };

## 4. Enable it via the module

In a module you already import (e.g. modules/cosmic-applets.nix or
modules/desktop-cosmic.nix), add the module import and enable it. Two ways:

**Option A - use the provided NixOS module (installs the package):**

    imports = [ inputs.cosmic-cassette-deck.nixosModules.default ];
    services.cosmic-cassette-deck.enable = true;

**Option B - just add the package directly (like easyspeak in your flake):**

    environment.systemPackages = [
      inputs.cosmic-cassette-deck.packages.${pkgs.system}.default
    ];

## 5. Rebuild

    sudo nixos-rebuild switch --flake ~/nixos-config#<your-host>

The FIRST build will FAIL with a cargoHash mismatch - this is expected (same as
your other packages). Copy the `got: sha256-...` value from the error into
`packages/cosmic-cassette-deck/flake.nix`, replacing `lib.fakeHash` on the
`cargoHash =` line, then rebuild again.

## 6. Add the applet to the panel

    # restart the panel so it picks up the new desktop entry
    pkill cosmic-panel

Then: COSMIC Settings -> Desktop -> Panel -> Add Applet -> "Cassette Deck".
Click the cassette icon in the panel -> the player opens.

## Notes

- Both binaries (`cosmic-cassette-deck`, `cosmic-cassette-applet`) install into
  the Nix store and are wrapped with the right LD_LIBRARY_PATH, so no PATH
  problems - the panel launches them from the store path in the desktop entry.
- The desktop entry uses the COSMIC applet fields (Categories=COSMIC,
  X-CosmicApplet=true, etc) matching your working applets.
- Clicking the applet spawns the player. Single-instance (raise instead of
  duplicate) isn't wired - ask if you want it.
