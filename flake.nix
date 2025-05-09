{
  description = "Nix version differ";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default-linux";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs: let
    inherit (inputs.nixpkgs) lib;
    systems = [
      "x86_64-linux"
      "aarch64-linux"
      "x86_64-darwin"
      "aarch64-darwin"
    ];
    eachSystem = lib.genAttrs systems;
    pkgsFor = eachSystem (system:
      import inputs.nixpkgs {
        localSystem.system = system;
        overlays = [(import inputs.rust-overlay)];
      });
  in {
    devShells = eachSystem (system: {
      default = pkgsFor.${system}.mkShell {
        packages = builtins.attrValues {
          inherit
            (pkgsFor.${system}.rust-bin.nightly.latest)
            cargo
            rustc
            rustfmt
            clippy
            ;
        };
      };
    });
  };
}
