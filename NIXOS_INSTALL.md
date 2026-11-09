# Installing on NixOS

There are three ways to install it, fastest first. All three build the app from source, because there's no binary cache for this repo. The first build also builds libcosmic and takes several minutes. After that it's kept in your Nix store.

**Flakes needed:** all three need flakes turned on (`experimental-features = nix-command flakes`).

## Just try it

Runs the app without installing anything:

```sh
nix run github:ctsdownloads/cosmic-cassette-deck
```

Nothing is added to your system and nothing shows up in your app menu.

## Install it for your user

One command, no config to edit:

```sh
nix profile add github:ctsdownloads/cosmic-cassette-deck
```

**Older Nix:** before `add`, the command was `nix profile install`. Use that if `add` isn't recognized.

It shows up in your app menu as **Cassette Deck**.

- **Update:** `nix profile upgrade --all`
- **List what you have:** `nix profile list`
- **Remove:** `nix profile remove` plus the name from that list

**Not declarative:** it lives in your user profile, not your system configuration. It isn't in git and won't come back when you rebuild a machine from your flake. If that matters to you, use the next section.

## Install it declaratively (the NixOS way)

This puts the app in your system configuration. It's versioned with the rest of your setup, applies to every user, and survives a rebuild from scratch. Most NixOS users will want this.

### 1. Add it as a flake input

In your system `flake.nix`, next to your other inputs:

```nix
cosmic-cassette-deck = {
  url = "github:ctsdownloads/cosmic-cassette-deck";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

**Working on the code?** Point at your checkout instead: `url = "path:/path/to/cosmic-cassette-deck";`.

### 2. Turn it on

Do this in `flake.nix` itself, where the input is already available. Add it to the arguments of `outputs`:

```nix
outputs = { self, nixpkgs, cosmic-cassette-deck, ... }:
```

Then add two entries to your host's `modules` list:

```nix
modules = [
  ./configuration.nix
  cosmic-cassette-deck.nixosModules.default
  { services.cosmic-cassette-deck.enable = true; }
];
```

Nothing needs adding to `configuration.nix`.

**Just the package, no module:** add this to the same `modules` list instead:

```nix
({ pkgs, ... }: {
  environment.systemPackages = [
    cosmic-cassette-deck.packages.${pkgs.stdenv.hostPlatform.system}.default
  ];
})
```

#### In configuration.nix instead

This works only if your flake passes its inputs down to your modules, meaning your `nixpkgs.lib.nixosSystem` call has `specialArgs = { inherit inputs; };`. Without it, the rebuild fails with `error: undefined variable 'inputs'`. If you have it:

```nix
imports = [ inputs.cosmic-cassette-deck.nixosModules.default ];
services.cosmic-cassette-deck.enable = true;
```

### 3. Rebuild

Rebuild the way you normally do, pointing at your system flake:

```sh
sudo nixos-rebuild switch --flake /path/to/your/flake
```

Nix picks the configuration that matches your hostname. If yours has a different name, add it: `--flake /path/to/your/flake#that-name`.

**Update later:** `nix flake update cosmic-cassette-deck`, then rebuild.

## Launch it

Open **Cassette Deck** from the COSMIC app library or your app menu. There's no PATH or environment setup to do. The app is wrapped with everything it needs to find its libraries and icons.

## Good to know

- **Chips:** builds for `x86_64-linux` and `aarch64-linux`. ARM hasn't been tested.
- **Other desktops:** on GNOME, KDE Plasma and the rest, the Adwaita icons are added for you, so the window's minimize, maximize and close buttons don't show up blank.
- **Other distros:** `nix run` and `nix profile add` work on any distro with Nix installed. The declarative route is NixOS only.
- **Changing dependencies:** they come from the committed `Cargo.lock`, so there's no `cargoHash` to update. If you change dependencies in a fork, run `cargo generate-lockfile` inside `nix develop` and commit the new `Cargo.lock`.
