{
  description = "cosmic-cassette-deck package + NixOS module";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      mkModule = { config, lib, pkgs, ... }: {
        options.services.cosmic-cassette-deck = {
          enable = lib.mkEnableOption "COSMIC photorealistic cassette player";
        };

        config = lib.mkIf config.services.cosmic-cassette-deck.enable {
          environment.systemPackages = [
            self.packages.${pkgs.system}.default
          ];
        };
      };
    in
    {
      nixosModules.default = mkModule;
      nixosModules.cosmic-cassette-deck = mkModule;
    }
    // flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        lib = pkgs.lib;

        # Linked at build time: alsa (rodio). Everything else below is
        # dlopen'd at runtime by winit/wgpu and provided via LD_LIBRARY_PATH.
        runtimeLibs = with pkgs; [
          libxkbcommon
          wayland
          vulkan-loader
          libGL
          alsa-lib
        ];
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "cosmic-cassette-deck";
          version = "0.0.6";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
            autoPatchelfHook
            makeBinaryWrapper
          ];

          # runtimeLibs are dlopen'd (wrapped into LD_LIBRARY_PATH below);
          # stdenv.cc.cc.lib supplies libgcc_s.so.1, which the Rust binary links
          # and autoPatchelfHook must resolve at build time.
          buildInputs = runtimeLibs ++ [ pkgs.stdenv.cc.cc.lib ];

          postInstall = ''
            install -Dm644 res/io.github.ctsdownloads.CosmicCassetteDeck.desktop \
              $out/share/applications/io.github.ctsdownloads.CosmicCassetteDeck.desktop
            install -Dm644 res/io.github.ctsdownloads.CosmicCassetteDeck.svg \
              $out/share/icons/hicolor/scalable/apps/io.github.ctsdownloads.CosmicCassetteDeck.svg
          '';

          postFixup = ''
            wrapProgram $out/bin/cosmic-cassette-deck \
              --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
          '';

          meta = {
            description = "Photorealistic COSMIC cassette player with a fanned-case browser";
            homepage = "https://github.com/ctsdownloads/cosmic-cassette-deck";
            license = lib.licenses.gpl3Only;
            platforms = lib.platforms.linux;
            mainProgram = "cosmic-cassette-deck";
          };
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
          packages = with pkgs; [ rustc cargo just rust-analyzer ];
          LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
        };
      });
}
