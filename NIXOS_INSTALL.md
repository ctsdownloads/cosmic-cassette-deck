# Installing on NixOS

This builds the app as a Nix package and installs its launcher, so it appears in the COSMIC app library (and any freedesktop app menu) like any other application.

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

Wire it up in `flake.nix` itself, where the input is already in scope. Take it as an argument to `outputs`:

```nix
outputs = { self, nixpkgs, cosmic-cassette-deck, ... }:
```

then add two entries to your host's `modules` list:

```nix
modules = [
  ./configuration.nix
  cosmic-cassette-deck.nixosModules.default
  { services.cosmic-cassette-deck.enable = true; }
];
```

That's it - nothing to add to `configuration.nix`.

If you would rather skip the module and just install the package, add this to the same `modules` list instead:

```nix
({ pkgs, ... }: {
  environment.systemPackages = [
    cosmic-cassette-deck.packages.${pkgs.stdenv.hostPlatform.system}.default
  ];
})
```

### Putting it in configuration.nix instead

This also works, but only if your flake already passes its inputs down to your modules - that is, your `nixpkgs.lib.nixosSystem` call includes `specialArgs = { inherit inputs; };`. Without it, `inputs` is not a module argument and the rebuild fails with `error: undefined variable 'inputs'`. If you have it:

```nix
imports = [ inputs.cosmic-cassette-deck.nixosModules.default ];
services.cosmic-cassette-deck.enable = true;
```

## 3. Rebuild

Rebuild the way you normally do, pointing at wherever your system flake lives:

```sh
sudo nixos-rebuild switch --flake /path/to/your/flake
```

Nix picks the configuration matching your hostname. If your `nixosConfigurations` attribute is named something else, name it explicitly: `--flake /path/to/your/flake#that-name`.

Dependencies come from the committed `Cargo.lock` (`cargoLock.lockFile`), so there is no `cargoHash` to chase. If you fork and change dependencies, regenerate the lock with `cargo generate-lockfile` (inside `nix develop`) and commit it.

## 4. Launch it

The binary installs into the Nix store and is wrapped with everything it needs to find its libraries and icons, so there is no PATH or environment setup to do. Open it from the COSMIC app library, or your app menu, as "Cassette Deck".

## Notes

- Builds for `x86_64-linux` and `aarch64-linux`.
- The desktop entry is a standard application launcher (`Type=Application`, audio/player categories).
- One binary is produced, `cosmic-cassette-deck`; the app menu launches it from its Nix store path.
- Outside COSMIC (GNOME, KDE Plasma, and the rest), the wrapper adds the Adwaita icon theme to `XDG_DATA_DIRS`, so the window's minimize, maximize, and close buttons render. The COSMIC icon theme those buttons normally come from isn't installed on other desktops, and without this they draw blank.
