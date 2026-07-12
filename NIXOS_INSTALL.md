# Installing on NixOS

Three ways in, fastest first. All three compile the app from source - there is no binary cache for this repo, so the first build pulls in libcosmic and takes several minutes. After that it is cached in your Nix store.

All of these need flakes enabled (`experimental-features = nix-command flakes`).

## Just try it

Runs the app without installing anything:

```sh
nix run github:ctsdownloads/cosmic-cassette-deck
```

Nothing is added to your system, nothing appears in your app menu. Good for a look before you commit.

## Install it for your user

One command, no config edits:

```sh
nix profile install github:ctsdownloads/cosmic-cassette-deck
```

It lands in your app menu as **Cassette Deck**, same as any other app. Update it with `nix profile upgrade`, list what you have with `nix profile list`, and remove it with `nix profile remove` plus the name from that list.

The catch: this is **not declarative**. It lives in your user profile, not in your system configuration, so it is not in git and won't come back if you rebuild a machine from your flake. If that matters to you, use the next section instead.

## Install it declaratively (the NixOS way)

Puts the app in your system configuration, so it is versioned with the rest of your setup, applies to every user, and survives a rebuild from scratch. This is what most NixOS users will want.

### 1. Add it as a flake input

In your system `flake.nix`, alongside your other inputs:

```nix
cosmic-cassette-deck = {
  url = "github:ctsdownloads/cosmic-cassette-deck";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

For local hacking, point at a checkout instead: `url = "path:/path/to/cosmic-cassette-deck";`.

### 2. Enable it

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

#### Putting it in configuration.nix instead

This also works, but only if your flake already passes its inputs down to your modules - that is, your `nixpkgs.lib.nixosSystem` call includes `specialArgs = { inherit inputs; };`. Without it, `inputs` is not a module argument and the rebuild fails with `error: undefined variable 'inputs'`. If you have it:

```nix
imports = [ inputs.cosmic-cassette-deck.nixosModules.default ];
services.cosmic-cassette-deck.enable = true;
```

### 3. Rebuild

Rebuild the way you normally do, pointing at wherever your system flake lives:

```sh
sudo nixos-rebuild switch --flake /path/to/your/flake
```

Nix picks the configuration matching your hostname. If your `nixosConfigurations` attribute is named something else, name it explicitly: `--flake /path/to/your/flake#that-name`.

Dependencies come from the committed `Cargo.lock` (`cargoLock.lockFile`), so there is no `cargoHash` to chase. If you fork and change dependencies, regenerate the lock with `cargo generate-lockfile` (inside `nix develop`) and commit it.

To update later, bump the input and rebuild: `nix flake update cosmic-cassette-deck`.

## Launch it

The binary installs into the Nix store and is wrapped with everything it needs to find its libraries and icons, so there is no PATH or environment setup to do. Open it from the COSMIC app library, or your app menu, as "Cassette Deck".

## Notes

- Builds for `x86_64-linux` and `aarch64-linux`.
- The desktop entry is a standard application launcher (`Type=Application`, audio/player categories).
- One binary is produced, `cosmic-cassette-deck`; the app menu launches it from its Nix store path.
- Outside COSMIC (GNOME, KDE Plasma, and the rest), the wrapper adds the Adwaita icon theme to `XDG_DATA_DIRS`, so the window's minimize, maximize, and close buttons render. The COSMIC icon theme those buttons normally come from isn't installed on other desktops, and without this they draw blank.
- `nix run` and `nix profile install` work on any distro with Nix installed, not just NixOS. The declarative route is NixOS-only.
