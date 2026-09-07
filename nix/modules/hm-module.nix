self:
{ pkgs, lib, ... }:
{
  imports = [ ./hm/sonora.nix ];
  programs.sonora.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.default;
}
