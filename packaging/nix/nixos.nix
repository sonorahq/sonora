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
    package = cfg.package;
    inherit (cfg) settings;
  };
in
{
  _class = "nixos";
  imports = [ ./options.nix ];

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ settings.package ];
  };
}
