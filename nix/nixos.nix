{
  config,
  pkgs,
  lib,
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
  _class = "nixos";
  imports = [ ./options.nix ];

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ settings.package ];
  };
}
