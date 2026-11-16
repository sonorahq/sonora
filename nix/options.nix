{ lib, pkgs, ... }:
{
  options.programs.sonora = {
    enable = lib.mkEnableOption "Sonora, a native, familiar music streaming client";

    package = lib.mkOption {
      type = lib.types.package;
    };

    settings = lib.mkOption {
      type = (pkgs.formats.json { }).type;
      default = { };
      example = {
        gapless = true;
        appearance = {
          theme = "dark";
          icons = "solar";
        };
      };
      description = ''
        Configuration written to {file}`$XDG_CONFIG_HOME/sonora/settings.json`.
        Same JSON keys as the app. Set keys are merged on switch and each launch;
        omitted keys, including resume and window bounds, are kept.
      '';
    };
  };
}
