# Non-flake entry point: `nix-shell` in the project root.
# Mirrors flake.nix devShell for classic NixOS config workflows.
{ pkgs ? import <nixpkgs> { } }:

let
  runtimeLibs = with pkgs; [
    libxkbcommon   # winit keyboard handling (dlopen'd at runtime)
    wayland        # COSMIC is Wayland-native
    vulkan-loader  # wgpu backend
    libGL          # GL fallback path
    alsa-lib       # rodio/cpal audio output
  ];
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ cargo rustc rustfmt clippy pkg-config ];
  buildInputs = runtimeLibs;

  # winit/wgpu/cpal dlopen these instead of linking; without this the
  # binary builds fine and then dies at startup unable to find libwayland.
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;

  # Bake the store paths into the binary as RPATH so it runs standalone;
  # dlopen (winit/wayland, vulkan) searches the caller's RUNPATH.
  RUSTFLAGS = "-C link-arg=-Wl,-rpath,${pkgs.lib.makeLibraryPath runtimeLibs}";
}
