{
  description = "Dix - Diff Nix";

  nixConfig = {
    extra-substituters = [
      "https://dix.cachix.org/"
    ];

    extra-trusted-public-keys = [
      "dix.cachix.org-1:8zQJZGvlOLYwlSCY/gVY14rqL8taVslOVbtT0jZFDGk="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    systems.url = "github:nix-systems/default";

    crane.url = "github:ipetkov/crane";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = inputs @ { self, nixpkgs, systems, ... }: let
    inherit (nixpkgs) lib;

    eachSystem = lib.genAttrs (import systems);

    pkgsFor = eachSystem (system: import nixpkgs {
      inherit system;

      overlays = [
        inputs.fenix.overlays.default

        (self: _: {
          crane = (inputs.crane.mkLib self).overrideToolchain (self.fenix.combine (lib.attrValues {
            inherit (self.fenix.stable)
              cargo
              clippy
              rust-analyzer
              rustc
            ;

            # Nightly rustfmt for the formatting options.
            inherit (self.fenix.default)
              rustfmt
            ;
          }));

          dix = {
            src = self.crane.cleanCargoSource ./.;

            cargoArguments = {
              inherit (self.dix) src;

              strictDeps = true;
            };

            cargoArtifacts = self.crane.buildDepsOnly self.dix.cargoArguments;
          };
        })
      ];
    });
  in {
    packages = eachSystem (system: let pkgs = pkgsFor.${system}; in {
      default = self.packages.${system}.dix;

      dix = pkgs.crane.buildPackage (pkgs.dix.cargoArguments // {
        inherit (pkgs.dix) cargoArtifacts;

        pname = "dix";
        cargoExtraArgs = "--package dix";

        doCheck = false;
      });
    });

    devShells = eachSystem (system: let pkgs = pkgsFor.${system}; in {
      default = self.devShells.${system}.dix;

      dix = pkgs.crane.devShell {
        packages = lib.attrValues {
          inherit (pkgs)
            # A nice compiler daemon.
            bacon

            # Better tests.
            cargo-nextest

            # TOML formatting.
            taplo
          ;
        };

        # For some reason rust-analyzer doesn't pick it up sometimes.
        env.CLIPPY_CONF_DIR = pkgs.writeTextDir "clippy.toml" (lib.readFile ./.clippy.toml);

        shellHook = ''
          # So we can do `dix` instead of `./target/debug/dix`
          root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
          export PATH="$PATH":"$root/target/debug"
        '';
      };
    });

    checks = eachSystem (system: let pkgs = pkgsFor.${system}; in {
      inherit (self.packages.${system}) dix;

      dix-doctest = pkgs.crane.cargoDocTest (pkgs.dix.cargoArguments // {
        inherit (pkgs.dix) cargoArtifacts;
      });

      dix-nextest = pkgs.crane.cargoNextest (pkgs.dix.cargoArguments // {
        inherit (pkgs.dix) cargoArtifacts;
      });

      dix-clippy = pkgs.crane.cargoClippy (pkgs.dix.cargoArguments // {
        inherit (pkgs.dix) cargoArtifacts;

        env.CLIPPY_CONF_DIR = pkgs.writeTextDir "clippy.toml" (lib.readFile ./.clippy.toml);

        cargoClippyExtraArgs = "--all-targets -- --deny warnings";
      });

      dix-doc = pkgs.crane.cargoDoc (pkgs.dix.cargoArguments // {
        inherit (pkgs.dix) cargoArtifacts;
      });

      dix-fmt = pkgs.crane.cargoFmt {
        inherit (pkgs.dix) src;

        rustFmtExtraArgs = "--config-path ${./.rustfmt.toml}";
      };

      dix-toml-fmt = pkgs.crane.taploFmt {
        src = lib.sources.sourceFilesBySuffices pkgs.dix.src [ ".toml" ];

        taploExtraArgs = "--config ${./.taplo.toml}";
      };

      dix-audit = pkgs.crane.cargoAudit {
        inherit (inputs) advisory-db;
        inherit (pkgs.dix) src;
      };

      dix-deny = pkgs.crane.cargoDeny {
        inherit (pkgs.dix) src;

        cargoDenyChecks = "bans licenses sources --config ${./.deny.toml}";
      };
    });
  };
}
