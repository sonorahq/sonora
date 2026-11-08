{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.sonora;
  settings = import ./settings.nix {
    inherit pkgs lib;
    inherit (cfg) package settings;
  };
in
{
  _class = "homeManager";
  imports = [ ./options.nix ];

  config = lib.mkIf cfg.enable {
    home.packages = [ settings.package ];

    home.activation.sonoraSettings = lib.mkIf (cfg.settings != { }) (
      lib.hm.dag.entryAfter [ "writeBoundary" ] ''
        $DRY_RUN_CMD ${settings.merge}
      ''
    );
  };
}
