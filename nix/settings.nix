{
  pkgs,
  lib,
  package,
  settings,
}:
let
  json = pkgs.formats.json { };
  configHome =
    if pkgs.stdenv.hostPlatform.isDarwin then
      "$HOME/Library/Application Support"
    else
      "\${XDG_CONFIG_HOME:-$HOME/.config}";
  generated = json.generate "sonora-settings.json" ({ version = 1; } // settings);
  apply = pkgs.writeShellApplication {
    name = "sonora-apply-settings";
    runtimeInputs = [ pkgs.jq ];
    text = ''
      generated=$1
      dest="${configHome}/sonora/settings.json"
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
  wrapped = pkgs.symlinkJoin {
    name = "sonora";
    paths = [ package ];
    nativeBuildInputs = [ pkgs.makeWrapper ];
    postBuild = ''
      rm -f $out/bin/sonora
      makeWrapper ${lib.getExe package} $out/bin/sonora \
        --run ${lib.escapeShellArg merge}
    '';
  };
in
{
  package = if settings == { } then package else wrapped;
  inherit merge;
}
