{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
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
        runtimeDependencies = with pkgs; [
          libxkbcommon
          wayland
          mesa
          libGL
          libglvnd
          vulkan-loader
          udev
          openblas
          dbus
        ];
        runtimeDependenciesPath = pkgs.lib.makeLibraryPath runtimeDependencies;
        buildInputs = with pkgs; [
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
      in
      with pkgs;
      {
        packages = rec {
          default = rstrf;
          rstrf = rustPlatform.buildRustPackage {
            pname = "rstrf";
            version = "0.1.0";
            src = ./.;

            nativeBuildInputs = [
              pkg-config
              copyDesktopItems
            ];
            inherit buildInputs;

            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "plotters-iced2-0.14.0" = "sha256-STEowCxh3PbSWhxGEcIAUxM3uXwdFaM7GKjpW666uSg=";
                "sgp4-2.3.0" = "sha256-xxv1P3V1v7Y37DHGQrR/Vrk9jofKDB37nsDhygQkmnM=";
                "space_track-0.1.0" = "sha256-P69yU53a5MIMvBvmzXfr+S+14F21pQPUvHema7skqp4=";
              };
            };

            postFixup = ''
              rpath=$(patchelf --print-rpath $out/bin/rstrf)
              patchelf --set-rpath "$rpath:${runtimeDependenciesPath}" $out/bin/rstrf
            '';

            postInstall = ''
              for d in 32x32 64x64 128x128; do
                install -Dm644 resources/icons/hicolor/$d/apps/de.jazzpi.rstrf.png \
                  $out/share/icons/hicolor/$d/apps/de.jazzpi.rstrf.png
              done
              install -Dm644 resources/icons/hicolor/scalable/apps/de.jazzpi.rstrf.svg \
                $out/share/icons/hicolor/scalable/apps/de.jazzpi.rstrf.svg
            '';

            desktopItems = [ ./resources/de.jazzpi.rstrf.desktop ];

            meta =
              let
                inherit (lib) licenses platforms;
              in
              {
                description = "Oxidized RF Satellite Tracking";
                homepage = "https://github.com/jazzpi/rstrf";
                license = licenses.gpl3Only;
                platforms = platforms.linux;
              };
          };
        };
        # Adapted from https://wiki.nixos.org/wiki/Rust#Installation_via_rustup
        devShells.default = mkShell rec {
          strictDeps = true;
          nativeBuildInputs = [
            rustup
            rustPlatform.bindgenHook
            pkg-config
          ];
          inherit buildInputs;
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
              export __EGL_VENDOR_LIBRARY_FILENAMES=${mesa}/share/glvnd/egl_vendor.d/50_mesa.json
              export RUST_LOG="warn,rstrf=debug,cosmic_config::dbus=off"
              export RUST_BACKTRACE=1
            '';
        };
      }
    );
}
