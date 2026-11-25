{
  description = "A native, familiar music streaming client";

  # CI builds the `stable` and `nightly` branches and every push to main for Linux and Apple
  # silicon macOS and pushes the result here, so the package downloads instead of compiling.
  nixConfig = {
    extra-substituters = [ "https://sonora.cachix.org" ];
    extra-trusted-public-keys = [
      "sonora.cachix.org-1:qbPeDsvZPbwUWHpQS5Oe0bjz5oHAVscHem+MCzwrc9Q="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
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

      # Copies only what cargo reads out of a source tree. The result depends on file contents
      # alone, so a commit that touches only docs, CI or this file keeps the cached build.
      cargoSource =
        root:
        builtins.path {
          name = "source";
          path = root;
          filter =
            path: _:
            builtins.elem (lib.head (lib.splitString "/" (lib.removePrefix "${toString root}/" path))) [
              ".cargo"
              "Cargo.toml"
              "Cargo.lock"
              "crates"
              "assets"
              "COPYING"
              "THIRD-PARTY.md"
            ];
        };

      # The nightly's version. Only the commit the nightly workflow puts on the `nightly` branch
      # carries this file, and its presence makes the package a nightly build.
      nightly = lib.optionalAttrs (builtins.pathExists ./nix/nightly.json) (
        lib.importJSON ./nix/nightly.json
      );
      toolchainChannel = (lib.importTOML ./rust-toolchain.toml).toolchain.channel;
    in
    {
      packages = forEachSystem (
        pkgs:
        let
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
                webkitgtk_4_1
                glib-networking
              ]
            else
              [ ];

          inherit (pkgs.stdenv.hostPlatform) system;
          rustToolchain = pkgs.rust-bin.stable.${toolchainChannel}.minimal;

          # WebKit plays a page's media through GStreamer and aborts its web process when no
          # audio sink element exists. The Nix webkitgtk closure carries only core and base, and
          # autoaudiosink lives in good, so the plugin path has to name all three.
          gstPluginPath = pkgs.lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" (
            with pkgs.gst_all_1;
            [
              gstreamer
              gst-plugins-base
              gst-plugins-good
            ]
          );

          # WebKit reads its font settings from org.gnome.desktop.interface, and a session without
          # GNOME lists no schema for it. The defaults it falls back on mismeasure text, which on
          # Google's sign-in page collapsed the whole layout into one narrow column.
          gsettingsSchemas = lib.concatMapStringsSep ":" (
            package: "${package}/share/gsettings-schemas/${package.name}"
          ) (
            with pkgs;
            [
              gsettings-desktop-schemas
              gtk3
            ]
          );

          alsaPluginDirectory = pkgs.symlinkJoin {
            name = "sonora-alsa-plugins";
            paths = [
              "${pkgs.pipewire}/lib/alsa-lib"
              "${pkgs.alsa-plugins}/lib/alsa-lib"
            ];
          };

          # What Soloist loads besides glibc: libatomic when it starts, and PipeWire or PulseAudio
          # once it opens a sound device.
          soloistLibraries = lib.makeLibraryPath (
            with pkgs;
            [
              stdenv.cc.cc.lib
              pipewire
              libpulseaudio
            ]
          );

          # The source build uses the toolchain the devShell and CI pin, not whatever rustc
          # nixpkgs carries that day.
          craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

          # Points the Linux binary at its runtime libraries and environment, since webkitgtk and
          # the GPU and audio stacks are opened with dlopen and never show up as link-time
          # dependencies.
          linuxFixup = lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
            patchelf \
              --set-interpreter "${pkgs.stdenv.cc.bintools.dynamicLinker}" \
              --add-rpath "${lib.makeLibraryPath (runtimeLibraries ++ [ pkgs.stdenv.cc.cc.lib ])}" \
              "$out/bin/sonora"
            wrapProgram "$out/bin/sonora" \
              --set ALSA_PLUGIN_DIR ${alsaPluginDirectory} \
              --prefix GIO_EXTRA_MODULES : ${pkgs.glib-networking}/lib/gio/modules \
              --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${gstPluginPath} \
              --prefix XDG_DATA_DIRS : ${gsettingsSchemas}
          '';

          src = cargoSource ./.;
          cargoVersion = (lib.importTOML "${src}/Cargo.toml").workspace.package.version;

          # gpui_apple's build script reads gpui's sources from `../gpui`, but a vendored git crate sits
          # in `<name>-<version>`, so on macOS the path is pointed at the vendored gpui instead.
          cargoVendorDir = craneLib.vendorCargoDeps {
            inherit src;
            overrideVendorGitCheckout =
              packages: drv:
              let
                gpui = lib.findFirst (package: package.name == "gpui") null packages;
              in
              if pkgs.stdenv.hostPlatform.isDarwin && gpui != null then
                drv.overrideAttrs (_: {
                  postPatch = ''
                    substituteInPlace crates/gpui_apple/build.rs \
                      --replace-fail '"../gpui"' '"../gpui-${gpui.version}"'
                  '';
                })
              else
                drv;
          };

          # What both the dependency build and the app build need to compile. Nothing in here may
          # depend on the app's own code or version, or every commit would rebuild the
          # dependencies too.
          commonArgs = {
            inherit src;
            inherit cargoVendorDir;
            pname = "sonora";
            version = cargoVersion;
            strictDeps = true;
            doCheck = false;
            # The Nix Apple SDK has no Metal compiler, so on macOS gpui compiles its shaders at launch
            # instead of reaching for Xcode at build time.
            cargoExtraArgs =
              "--locked --package sonora"
              + lib.optionalString pkgs.stdenv.hostPlatform.isDarwin " --features runtime-shaders";

            nativeBuildInputs =
              with pkgs;
              lib.flatten [
                cmake
                pkg-config
                (lib.optionals stdenv.hostPlatform.isLinux [
                  makeWrapper
                  mold
                ])
              ];
            buildInputs =
              with pkgs;
              lib.flatten [
                (lib.optionals stdenv.hostPlatform.isLinux [
                  dbus
                  fontconfig
                  libxcb
                  libxkbcommon
                  libx11
                  pipewire
                  stdenv.cc.cc.lib
                  (alsa-lib-with-plugins.override {
                    plugins = [
                      alsa-plugins
                      pipewire
                    ];
                  })
                ])
                (lib.optionals stdenv.hostPlatform.isDarwin [
                  apple-sdk_15
                  (darwinMinVersionHook "10.15")
                ])
              ];
          };

          # Every dependency, compiled once per Cargo.lock and pushed to the cache on its own, so a
          # commit that only changes the app compiles just the workspace. Only the build runs: a
          # `cargo check` pass would be thrown away, since nothing here runs clippy.
          cargoArtifacts = craneLib.buildDepsOnly (
            commonArgs
            // {
              buildPhaseCargoCommand = "cargoWithProfile build ${commonArgs.cargoExtraArgs}";
            }
          );

          # Compiles the checked-out tree. On the `nightly` branch it takes the nightly's version, commit
          # and update channel, which the app reads at compile time.
          sonora = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              version = nightly.version or cargoVersion;
              nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ pkgs.installShellFiles ];

              # Soloist is Spotify's prebuilt binary and asks for a loader at /lib64, which NixOS
              # has only with nix-ld. The app starts it through the store's loader with these
              # instead.
              env =
                lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
                  SONORA_SOLOIST_LOADER = pkgs.stdenv.cc.bintools.dynamicLinker;
                  SONORA_SOLOIST_LIBRARIES = soloistLibraries;
                }
                // lib.optionalAttrs (nightly ? version) {
                  SONORA_VERSION = nightly.version;
                  SONORA_CHANNEL = "nightly";
                }
                // lib.optionalAttrs (nightly ? commit) {
                  SONORA_COMMIT = nightly.commit;
                };

              installPhase = ''
                runHook preInstall

                install -Dm755 target/release/sonora "$out/bin/sonora"
                ${
                  if pkgs.stdenv.hostPlatform.isDarwin then
                    ''
                      install -Dm755 target/release/sonora "$out/Applications/Sonora.app/Contents/MacOS/sonora"
                      install -Dm444 "$src/assets/macos/sonora.icns" \
                        "$out/Applications/Sonora.app/Contents/Resources/sonora.icns"
                      install -Dm444 "$src/assets/macos/Assets.car" \
                        "$out/Applications/Sonora.app/Contents/Resources/Assets.car"

                      sed \
                        "s/@VERSION@/$version/g" \
                        "$src/assets/macos/Info.plist" \
                        > "$out/Applications/Sonora.app/Contents/Info.plist"

                      LICENSE_DIR="$out/Applications/Sonora.app/Contents/Resources"
                    ''
                  else
                    ''
                      install -Dm444 "$src/assets/linux/sonora.desktop" \
                        "$out/share/applications/sonora.desktop"
                      install -Dm444 "$src/assets/linux/sonora.svg" \
                        "$out/share/icons/hicolor/scalable/apps/sonora.svg"
                      for icon in "$src"/assets/linux/icons/hicolor/*/apps/sonora.png; do
                        size="$(basename "$(dirname "$(dirname "$icon")")")"
                        install -Dm444 "$icon" \
                          "$out/share/icons/hicolor/$size/apps/sonora.png"
                      done

                      LICENSE_DIR="$out/share/licenses/sonora"
                    ''
                }
                install -Dm444 "$src/COPYING" "$LICENSE_DIR/LICENSE"
                install -Dm444 "$src/THIRD-PARTY.md" "$LICENSE_DIR/THIRD-PARTY.md"
                install -Dm444 "$src/assets/fonts/LICENSE.txt" \
                  "$LICENSE_DIR/sonora/LICENSE.Inter"
                for licence in "$src/assets/icons"/*/LICENSE; do
                  pack="$(basename "$(dirname "$licence")")"
                  install -Dm444 "$licence" \
                    "$LICENSE_DIR/icons/LICENSE.$pack"
                done
                install -Dm444 "$src/assets/icons/LICENSE" \
                  "$LICENSE_DIR/icons/LICENSE"

                ${lib.optionalString (pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform) ''
                  installShellCompletion --cmd sonora \
                    --bash <("$out/bin/sonora" completions bash) \
                    --zsh <("$out/bin/sonora" completions zsh) \
                    --fish <("$out/bin/sonora" completions fish)
                ''}
                runHook postInstall
              '';

              postFixup = linuxFixup;

              inherit meta;
            }
          );

          meta = {
            description = "A native, familiar music streaming client";
            mainProgram = "sonora";
            license = with lib.licenses; [
              gpl3Plus
              ofl
              isc
            ];
            platforms = lib.platforms.linux ++ lib.platforms.darwin;
          };

        in
        # The flake ref picks the channel: `github:sonorahq/sonora/stable` is the latest release,
        # `github:sonorahq/sonora/nightly` the nightly and `github:sonorahq/sonora` main.
        {
          inherit sonora;
          default = sonora;
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
                dioxus-cli
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
              # link still goes through Xcode's SDK.
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

      homeManagerModules = {
        default =
          { pkgs, lib, ... }:
          {
            imports = [ ./nix/home-manager.nix ];
            programs.sonora.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          };
        sonora = self.homeManagerModules.default;
      };

      homeModules = self.homeManagerModules;

      nixosModules = {
        default =
          { pkgs, lib, ... }:
          {
            imports = [ ./nix/nixos.nix ];
            programs.sonora.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          };
        sonora = self.nixosModules.default;
      };
    };
}
