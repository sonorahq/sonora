{
  description = "A native, familiar music streaming client";

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
      inherit (nixpkgs) lib;

      forEachSystem =
        fn:
        lib.genAttrs lib.systems.flakeExposed (
          system:
          let
            pkgs = import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            };
          in
          fn pkgs
        );
    in
    {
      packages = forEachSystem (
        pkgs:
        let
          toolchain =
            pkgs.rust-bin.stable.${(lib.importTOML ./rust-toolchain.toml).toolchain.channel}.minimal;
          sonora = pkgs.callPackage ./nix/package.nix {
            rustPlatform = pkgs.makeRustPlatform {
              cargo = toolchain;
              rustc = toolchain;
            };
            xcode =
              (import nixpkgs {
                inherit (pkgs.stdenv.hostPlatform) system;
                config.allowUnfree = true;
              }).darwin.xcode;
          };
        in
        {
          inherit sonora;
          default = sonora;
          sonora-src = sonora.override { fromSource = true; };
        }
        // lib.optionalAttrs (sonora.pname == "sonora-bin") {
          sonora-bin = sonora;
        }
        // lib.optionalAttrs (sonora.pname == "sonora-nightly") {
          sonora-bin = sonora.override { nightly = { }; };
          sonora-nightly = sonora;
        }
      );

      devShells = forEachSystem (
        pkgs:
        let
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          runtimeLibraries =
            with pkgs;
            if pkgs.stdenv.hostPlatform.isLinux then
              [
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
                openssl
                webkitgtk_4_1
                glib-networking
              ]
            else
              [ ];
          # Apple's own xcrun, so `xcrun metal` can reach the Metal toolchain that
          # Xcode 26 mounts outside DEVELOPER_DIR. The xcbuild shim the Apple SDK
          # drags onto PATH cannot, and neither can any xcrun pointed at the Nix SDK.
          xcodeXcrun = pkgs.runCommandLocal "xcode-xcrun" { } ''
            mkdir -p $out/bin
            ln -s /usr/bin/xcrun $out/bin/xcrun
          '';
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs =
              (with pkgs; [
                pkg-config
                cmake
                rustToolchain
                sccache
              ])
              ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ];

            buildInputs = runtimeLibraries;

            LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibraries;

            ALSA_PLUGIN_DIR =
              if pkgs.stdenv.hostPlatform.isLinux then
                "${pkgs.symlinkJoin {
                  name = "alsa-plugins-combined";
                  paths = [
                    "${pkgs.alsa-plugins}/lib/alsa-lib"
                    "${pkgs.pipewire}/lib/alsa-lib"
                  ];
                }}"
              else
                "";

            shellHook =
              # WebKit composites through EGL, which has to find the same drivers.
              pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                if [ ! -d /run/opengl-driver ]; then
                  export VK_DRIVER_FILES="${pkgs.mesa}/share/vulkan/icd.d"
                  export VK_IMPLICIT_LAYER_PATH="${pkgs.mesa}/share/vulkan/implicit_layer.d"
                  export __EGL_VENDOR_LIBRARY_DIRS="${pkgs.mesa}/share/glvnd/egl_vendor.d"
                  export LIBGL_DRIVERS_PATH="${pkgs.mesa}/lib/dri"
                  export GBM_BACKENDS_PATH="${pkgs.mesa}/lib/gbm"
                fi
                export GIO_EXTRA_MODULES="${pkgs.glib-networking}/lib/gio/modules''${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
                export GST_PLUGIN_SYSTEM_PATH_1_0="${
                  pkgs.lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" (
                    with pkgs.gst_all_1;
                    [
                      gstreamer
                      gst-plugins-base
                      gst-plugins-good
                    ]
                  )
                }''${GST_PLUGIN_SYSTEM_PATH_1_0:+:$GST_PLUGIN_SYSTEM_PATH_1_0}"
              ''
              # gpui_apple compiles its shaders with `xcrun -sdk macosx metal` at build
              # time. The Nix Apple SDK has no Metal toolchain, so hand xcrun back to the
              # installed Xcode. Clang follows DEVELOPER_DIR too and then reads Xcode's C
              # headers beside the Nix libc++, which breaks any C++ a build script compiles,
              # so C++ compiles are pinned to the Nix SDK through CXXFLAGS. Only compiles: the
              # link still has to see Xcode's SDK, which is where libsqlite3 comes from.
              + lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                # xcode-select echoes DEVELOPER_DIR back when it is set, so ask with it unset.
                if xcode="$(env -u DEVELOPER_DIR /usr/bin/xcode-select -p 2>/dev/null)"; then
                  export DEVELOPER_DIR="$xcode"
                  export PATH="${xcodeXcrun}/bin:$PATH"
                  export CXXFLAGS="''${CXXFLAGS:-} -isysroot $SDKROOT"
                fi
              '';
          };
        }
      );

      overlays.default = final: _prev: {
        sonora = self.packages.${final.stdenv.hostPlatform.system}.default;
      };

      nixosModules = {
        default = { pkgs, lib, ... }: {
          imports = [ ./nix/nixos.nix ];
          programs.sonora.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
        sonora = self.nixosModules.default;
      };

      homeManagerModules = {
        default = { pkgs, lib, ... }: {
          imports = [ ./nix/home-manager.nix ];
          programs.sonora.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
        sonora = self.homeManagerModules.default;
      };

      homeModules = self.homeManagerModules;
    };
}
