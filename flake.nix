{
  description = "Dix - Diff Nix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs: let
    eachSystem = inputs.nixpkgs.lib.genAttrs (import inputs.systems);
    pkgsFor = inputs.nixpkgs.legacyPackages;
  in {
    packages = eachSystem (system: {
      default = inputs.self.packages.${system}.dix;
      dix = pkgsFor.${system}.callPackage ./nix/package.nix {};
    });

    apps = eachSystem (system: let
      inherit (inputs.self.packages.${system}) dix;
    in {
      default = inputs.self.apps.${system}.dix;
      dix = {
        type = "app";
        program = "${dix}/bin/dix";
      };
    });

    devShells = eachSystem (system: {
      default = pkgsFor.${system}.mkShell {
        packages = builtins.attrValues {
          inherit
            (pkgsFor.${system})
            cargo
            rustc
            bacon
            ;
          inherit
            (pkgsFor.${system}.rustPackages)
            clippy
            ;

          inherit
            ((pkgsFor.${system}.extend
                inputs.rust-overlay.overlays.default).rust-bin.nightly.latest)
            rustfmt
            ;
        };
      };
    });
  };
}
