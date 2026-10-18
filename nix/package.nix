{
  pkgs,
  lib,
  rustPlatform,
  fromSource ? false,
  xcode ? pkgs.darwin.xcode,
  # Only the nightly tag carries this manifest.
  nightly ? if builtins.pathExists ./nightly.json then lib.importJSON ./nightly.json else { },
}:
let
  release = {
    version = "0.47.0";
    assets = {
      x86_64-linux = {
        target = "x86_64-unknown-linux-gnu";
        hash = "sha256-kIbkTKG/vChrlqjHDo768Sk1QD001ufIttBxR864qlM=";
      };
      aarch64-linux = {
        target = "aarch64-unknown-linux-gnu";
        hash = "sha256-fj4JvzLkuSabpTMhvd49plSMAVqXC6025atwg9BqF8I=";
      };
      aarch64-darwin = {
        target = "macos";
        hash = "sha256-eqxote68x3W95ONQFDZs49drLPeRVc2/jr8juR+IGqA=";
      };
    };
  };

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
        webkitgtk_4_1
        glib-networking
      ]
    else
      [ ];

  inherit (pkgs.stdenv.hostPlatform) system;

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

  alsaPluginDirectory = pkgs.symlinkJoin {
    name = "sonora-alsa-plugins";
    paths = [
      "${pkgs.pipewire}/lib/alsa-lib"
      "${pkgs.alsa-plugins}/lib/alsa-lib"
    ];
  };

  # Points the Linux binary at its runtime libraries and environment. Both packages run
  # it, since webkitgtk and the GPU and audio stacks are opened with dlopen and never
  # show up as link-time dependencies.
  linuxFixup = lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
    patchelf \
      --set-interpreter "${pkgs.stdenv.cc.bintools.dynamicLinker}" \
      --add-rpath "${lib.makeLibraryPath (runtimeLibraries ++ [ pkgs.stdenv.cc.cc.lib ])}" \
      "$out/bin/sonora"
    wrapProgram "$out/bin/sonora" \
      --set ALSA_PLUGIN_DIR ${alsaPluginDirectory} \
      --prefix GIO_EXTRA_MODULES : ${pkgs.glib-networking}/lib/gio/modules \
      --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${gstPluginPath}
  '';

  sonora-src = rustPlatform.buildRustPackage (final: {
    pname = "sonora";
    inherit ((lib.importTOML (final.src + /Cargo.toml)).workspace.package) version;

    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        ../Cargo.toml
        ../Cargo.lock
        ../.cargo
        ../crates
        ../assets
        ../COPYING
        ../THIRD-PARTY.md
      ];
    };
    cargoLock = {
      lockFile = final.src + /Cargo.lock;
      allowBuiltinFetchGit = true;
    };
    cargoBuildFlags = [
      "--package"
      "sonora"
    ];
    doCheck = false;

    nativeBuildInputs =
      with pkgs;
      lib.flatten [
        cmake
        pkg-config
        (lib.optionals stdenv.hostPlatform.isLinux [
          makeWrapper
          mold
        ])
        (lib.optionals stdenv.hostPlatform.isDarwin [
          (runCommandLocal "metal-shader-compiler" { } ''
            mkdir -p "$out/bin"
            ln -s ${xcode}/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/metal "$out/bin/metal"
            ln -s ${xcode}/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/metallib "$out/bin/metallib"
          '')
        ])
      ];
    buildInputs =
      with pkgs;
      lib.flatten [
        sqlite
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

    installPhase = ''
      runHook preInstall

      install -Dm755 "target/${pkgs.stdenv.hostPlatform.rust.cargoShortTarget}/$cargoBuildType/sonora" "$out/bin/sonora"
      ${
        if pkgs.stdenv.hostPlatform.isDarwin then
          ''
            install -Dm755 "target/${pkgs.stdenv.hostPlatform.rust.cargoShortTarget}/$cargoBuildType/sonora" "$out/Applications/Sonora.app/Contents/MacOS/sonora"
            install -Dm444 "$src/assets/macos/sonora.icns" \
              "$out/Applications/Sonora.app/Contents/Resources/sonora.icns"

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

      runHook postInstall
    '';

    postFixup = linuxFixup;

    inherit meta;
  });

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

  # Installs the binaries a GitHub release publishes under `tag`. Every asset is named
  # `sonora-<tag>-<target>`, and `assets` maps a Nix system to its target and hash.
  prebuilt =
    {
      pname,
      version,
      tag,
      assets,
    }:
    let
      asset = assets.${system};
    in
    pkgs.stdenv.mkDerivation {
      inherit pname version meta;

      src = pkgs.fetchurl {
        url = "https://github.com/sonorahq/sonora/releases/download/${tag}/sonora-${tag}-${asset.target}${
          if pkgs.stdenv.hostPlatform.isDarwin then ".dmg" else ""
        }";
        inherit (asset) hash;
      };

      dontUnpack = true;
      dontStrip = true;

      nativeBuildInputs =
        lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.makeWrapper ]
        ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
          pkgs.makeBinaryWrapper
          pkgs.undmg
        ];

      installPhase =
        if pkgs.stdenv.hostPlatform.isLinux then
          ''
            runHook preInstall
            install -Dm755 "$src" "$out/bin/sonora"
            install -Dm444 ${../assets/linux/sonora.desktop} \
              "$out/share/applications/sonora.desktop"
            install -Dm444 ${../assets/linux/sonora.svg} \
              "$out/share/icons/hicolor/scalable/apps/sonora.svg"
            for icon in ${../assets/linux/icons}/hicolor/*/apps/sonora.png; do
              size="$(basename "$(dirname "$(dirname "$icon")")")"
              install -Dm444 "$icon" \
                "$out/share/icons/hicolor/$size/apps/sonora.png"
            done
            install -Dm444 ${../COPYING} "$out/share/licenses/sonora/LICENSE"
            install -Dm444 ${../THIRD-PARTY.md} "$out/share/licenses/sonora/THIRD-PARTY.md"
            install -Dm444 ${../assets/fonts/LICENSE.txt} \
              "$out/share/licenses/sonora/LICENSE.Inter"
            for licence in ${../assets/icons}/*/LICENSE; do
              pack="$(basename "$(dirname "$licence")")"
              install -Dm444 "$licence" \
                "$out/share/licenses/sonora/icons/LICENSE.$pack"
            done
            install -Dm444 ${../assets/icons/LICENSE} \
              "$out/share/licenses/sonora/icons/LICENSE"
            runHook postInstall
          ''
        else
          ''
            runHook preInstall
            mnt="$(mktemp -d)"
            /usr/bin/hdiutil attach -readonly -nobrowse -mountpoint "$mnt" "$src"
            mkdir -p "$out/Applications" "$out/bin"
            cp -R "$mnt/Sonora.app" "$out/Applications/Sonora.app"
            /usr/bin/hdiutil detach "$mnt"
            makeBinaryWrapper \
              "$out/Applications/Sonora.app/Contents/MacOS/sonora" \
              "$out/bin/sonora"
            runHook postInstall
          '';

      postFixup = linuxFixup;
    };

  sonora-bin = prebuilt {
    pname = "sonora-bin";
    inherit (release) version assets;
    tag = "v${release.version}";
  };

  sonora-nightly = prebuilt {
    pname = "sonora-nightly";
    inherit (nightly) version tag assets;
  };
in
if fromSource then
  sonora-src
else if nightly ? assets && builtins.hasAttr system nightly.assets then
  sonora-nightly
else if builtins.hasAttr system release.assets then
  sonora-bin
else
  sonora-src
