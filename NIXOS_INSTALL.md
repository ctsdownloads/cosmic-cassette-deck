# Installing on NixOS

This builds the app as a Nix package and installs its launcher, so it appears in the COSMIC app library (and any freedesktop app menu) like any other application. It is a normal desktop app.

## 1. Add it as a flake input

In your system `flake.nix`, alongside your other inputs:

```nix
cosmic-cassette-deck = {
  url = "github:ctsdownloads/cosmic-cassette-deck";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

For local hacking, point at a checkout instead: `url = "path:/path/to/cosmic-cassette-deck";`.

## 2. Install the package

Either enable the provided NixOS module:

```nix
imports = [ inputs.cosmic-cassette-deck.nixosModules.default ];
services.cosmic-cassette-deck.enable = true;
```

or add the package directly:

```nix
environment.systemPackages = [
  inputs.cosmic-cassette-deck.packages.${pkgs.system}.default
];
```

## 3. Rebuild

```sh
sudo nixos-rebuild switch --flake /etc/nixos#your-host
```

Dependencies come from the committed `Cargo.lock` (`cargoLock.lockFile`), so there is no `cargoHash` to chase. If you fork and change dependencies, regenerate the lock with `cargo generate-lockfile` (inside `nix develop`) and commit it.

## 4. Launch it

The binary installs into the Nix store and is wrapped with the right `LD_LIBRARY_PATH`, so there is no PATH setup to do. Open it from the COSMIC app library, or your app menu, as "Cassette Deck".

## Notes

- The desktop entry is a standard application launcher (`Type=Application`, audio/player categories).
- One binary is produced, `cosmic-cassette-deck`; the app menu launches it from its Nix store path.
