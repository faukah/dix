{
  rustPlatform,
  lib,
  ...
}: let
  toml = (lib.importTOML ../Cargo.toml).package;
  pname = toml.name;
  inherit (toml) version;
in
  rustPlatform.buildRustPackage {
    inherit pname version;
    src = builtins.path {
      name = "${pname}-${version}";
      path = lib.sources.cleanSource ../.;
    };
    cargoLock.lockFile = ../Cargo.lock;
    doCheck = true;
  }
