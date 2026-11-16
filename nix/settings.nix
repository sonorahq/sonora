{
  lib,
  pkgs,
  package,
  settings,
}:
let
  json = pkgs.formats.json { };
  generated = json.generate "sonora-settings.json" ({ version = 1; } // settings);
  apply = pkgs.writeShellApplication {
    name = "sonora-apply-settings";
    runtimeInputs = [ pkgs.jq ];
    text = ''
      generated=$1
      dest="''${XDG_CONFIG_HOME:-$HOME/.config}/sonora/settings.json"
      mkdir -p "$(dirname "$dest")"
      if [[ -f "$dest" ]]; then
        tmp=$(mktemp "$dest.XXXXXX")
        trap 'rm -f "$tmp"' EXIT
        jq --indent 2 -s '.[0] * .[1]' "$dest" "$generated" > "$tmp"
        mv "$tmp" "$dest"
      else
        install -m600 "$generated" "$dest"
      fi
    '';
  };
  merge = "${lib.getExe apply} ${generated}";
  alsaPluginDir = pkgs.symlinkJoin {
    name = "sonora-alsa-plugins";
    paths = [
      "${pkgs.pipewire}/lib/alsa-lib"
      "${pkgs.alsa-plugins}/lib/alsa-lib"
    ];
  };
  wrapped = pkgs.symlinkJoin {
    name = "sonora";
    paths = [ package ];
    nativeBuildInputs = [ pkgs.makeWrapper ];
    postBuild = ''
      rm -f $out/bin/sonora
      makeWrapper ${lib.getExe package} $out/bin/sonora \
        ${lib.optionalString pkgs.stdenv.hostPlatform.isLinux "--set ALSA_PLUGIN_DIR ${alsaPluginDir}"} \
        ${lib.optionalString (settings != { }) "--run ${lib.escapeShellArg merge}"}
    '';
  };
in
{
  package = wrapped;
  inherit merge;
}
