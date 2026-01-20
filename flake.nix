# Adapted from https://wiki.nixos.org/wiki/Rust#Installation_via_rustup
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs =
    {
      nixpkgs,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };
      in
      with pkgs;
      {
        devShells.default = mkShell rec {
          strictDeps = true;
          nativeBuildInputs = [
            rustup
            rustPlatform.bindgenHook
            pkg-config
          ];
          buildInputs = [
            # Libraries here
            openssl
            libxkbcommon
            libGL
            wayland
            mesa
            vulkan-loader
            udev
            fontconfig
            openblas
          ];
          runtimeDependencies = [
            wayland
            mesa
            libGL
            libglvnd
            vulkan-loader
            udev
            openblas
          ];
          RUSTC_VERSION = "stable";
          shellHook =
            let
              toolchainPath = "\${RUSTUP_HOME:-$HOME/.rustup}/toolchains/${RUSTC_VERSION}-${stdenv.hostPlatform.rust.rustcTarget}";
              ldLibraryPath = lib.makeLibraryPath runtimeDependencies;
            in
            ''
              export PATH="${toolchainPath}/bin:$PATH"
              export RUSTC="${toolchainPath}/bin/rustc"
              export LD_LIBRARY_PATH="${ldLibraryPath}:$LD_LIBRARY_PATH"
              export WGPU_BACKEND=gl
              export __EGL_VENDOR_LIBRARY_FILENAMES=${mesa}/share/glvnd/egl_vendor.d/50_mesa.json
              export RUST_LOG="warn,rstrf=debug,cosmic_config::dbus=off"
              export RUST_BACKTRACE=1
            '';
        };
      }
    );
}
