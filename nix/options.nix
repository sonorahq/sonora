{ lib, pkgs, ... }:
{
  options.programs.sonora = {
    enable = lib.mkEnableOption "Sonora, a native, familiar music streaming client";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix { };
      defaultText = lib.literalExpression "pkgs.callPackage ./package.nix { }";
      description = "Sonora package to use.";
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
        Configuration written to {file}`$XDG_CONFIG_HOME/sonora/settings.json`
        on Linux or {file}`~/Library/Application Support/sonora/settings.json` on macOS.
        Same JSON keys as the app. Declared keys are merged before the
        {command}`sonora` command runs.
        Other keys are preserved. Home Manager also merges settings on activation.
      '';
    };
  };
}
