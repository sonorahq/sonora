{
  description = "Sonora - a native music streaming client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forEachSystem =
        fn:
        nixpkgs.lib.genAttrs systems (
          system:
          let
            pkgs = import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            };
          in
          fn pkgs
        );

      release = {
        version = "0.31.0";
        assets = {
          x86_64-linux = {
            target = "x86_64-unknown-linux-gnu";
            hash = "sha256-L4ehTKfTjDJTOGU3ahgVgPgz5V7/nhh4h+Og3lQbaNs=";
          };
          aarch64-linux = {
            target = "aarch64-unknown-linux-gnu";
            hash = "sha256-QpYMzemVKel9BfG+wzpkMq8MOeODPtXRZZHFbXDBvBI=";
          };
        };
      };
    in
    {
      packages = forEachSystem (
        pkgs:
        let
          runtimeLibraries = with pkgs; [
            vulkan-loader
            wayland
            libxkbcommon
            libxcb
            libx11
            libxcursor
            libxi
            fontconfig
            freetype
            alsa-lib
            dbus
            sqlite
          ];

          asset = release.assets.${pkgs.stdenv.hostPlatform.system};

          alsaPluginDirectory = pkgs.symlinkJoin {
            name = "sonora-alsa-plugins";
            paths = [
              "${pkgs.pipewire}/lib/alsa-lib"
              "${pkgs.alsa-plugins}/lib/alsa-lib"
            ];
          };

          sonora-bin = pkgs.stdenv.mkDerivation {
            pname = "sonora-bin";
            inherit (release) version;

            src = pkgs.fetchurl {
              url = "https://github.com/nolight132/sonora/releases/download/v${release.version}/sonora-v${release.version}-${asset.target}";
              inherit (asset) hash;
            };

            dontUnpack = true;
            dontStrip = true;

            nativeBuildInputs = [ pkgs.makeWrapper ];

            installPhase = ''
              runHook preInstall
              install -Dm755 "$src" "$out/bin/sonora"
              install -Dm444 ${./assets/linux/sonora.desktop} \
                "$out/share/applications/sonora.desktop"
              install -Dm444 ${./assets/linux/sonora.svg} \
                "$out/share/icons/hicolor/scalable/apps/sonora.svg"
              for icon in ${./assets/linux/icons}/hicolor/*/apps/sonora.png; do
                size="$(basename "$(dirname "$(dirname "$icon")")")"
                install -Dm444 "$icon" \
                  "$out/share/icons/hicolor/$size/apps/sonora.png"
              done
              install -Dm444 ${./COPYING} "$out/share/licenses/sonora/LICENSE"
              install -Dm444 ${./THIRD-PARTY.md} "$out/share/licenses/sonora/THIRD-PARTY.md"
              install -Dm444 ${./assets/fonts/LICENSE.txt} \
                "$out/share/licenses/sonora/LICENSE.Inter"
              for licence in ${./assets/icons}/*/LICENSE; do
                pack="$(basename "$(dirname "$licence")")"
                install -Dm444 "$licence" \
                  "$out/share/licenses/sonora/icons/LICENSE.$pack"
              done
              install -Dm444 ${./assets/icons/LICENSE} \
                "$out/share/licenses/sonora/icons/LICENSE"
              runHook postInstall
            '';

            postFixup = ''
              patchelf \
                --set-interpreter "${pkgs.stdenv.cc.bintools.dynamicLinker}" \
                --add-rpath "${pkgs.lib.makeLibraryPath (runtimeLibraries ++ [ pkgs.stdenv.cc.cc.lib ])}" \
                "$out/bin/sonora"
              wrapProgram "$out/bin/sonora" \
                --set ALSA_PLUGIN_DIR ${alsaPluginDirectory}
            '';

            meta = {
              description = "A native music streaming client, built with Rust and GPUI";
              mainProgram = "sonora";
              license = with pkgs.lib.licenses; [
                gpl3Plus
                ofl
                isc
              ];
              platforms = pkgs.lib.platforms.linux;
            };
          };
        in
        {
          inherit sonora-bin;
          sonora = sonora-bin;
          default = sonora-bin;
        }
      );

      devShells = forEachSystem (
        pkgs:
        let
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          runtimeLibraries = with pkgs; [
            vulkan-loader
            wayland
            libxkbcommon
            libxcb
            libx11
            libxcursor
            libxi
            fontconfig
            freetype
            alsa-lib
            dbus
            sqlite
          ];
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [
              mold
              pkg-config
              cmake
              rustToolchain
              sccache
            ];

            buildInputs = runtimeLibraries;

            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibraries;

            ALSA_PLUGIN_DIR = "${pkgs.symlinkJoin {
              name = "alsa-plugins-combined";
              paths = [
                "${pkgs.alsa-plugins}/lib/alsa-lib"
                "${pkgs.pipewire}/lib/alsa-lib"
              ];
            }}";

            shellHook = ''
              if [ ! -d /run/opengl-driver ]; then
                export VK_DRIVER_FILES="${pkgs.mesa}/share/vulkan/icd.d"
                export VK_IMPLICIT_LAYER_PATH="${pkgs.mesa}/share/vulkan/implicit_layer.d"
              fi
            '';
          };
        }
      );

      overlays.default = final: _prev: {
        sonora = self.packages.${final.stdenv.hostPlatform.system}.default;
      };

      homeManagerModules = {
        default = import ./nix/modules/hm-module.nix self;
        sonora = import ./nix/modules/hm-module.nix self;
      };

      homeModules = self.homeManagerModules;
    };
}
