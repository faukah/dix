{
  description = "Dix - Diff Nix";

  nixConfig = {
    extra-substituters = [
      "https://dix.cachix.io/"
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

  outputs = inputs @ {
    self,
    nixpkgs,
    systems,
    ...
  }: let
    inherit (nixpkgs) lib;

    eachSystem = lib.genAttrs (import systems);
    pkgsFor = eachSystem (system:
      import nixpkgs {
        inherit system;

        overlays = [
          inputs.fenix.overlays.default
          self.overlays.dix
          self.overlays.crane
        ];
      });
  in {
    overlays = {
      dix = final: _: {
        dix = {
          src = final.crane.cleanCargoSource ./.;

          cargoArguments = {
            inherit (final.dix) src;

            strictDeps = true;
          };

          cargoArtifacts = final.crane.buildDepsOnly final.dix.cargoArguments;
        };
      };

      crane = final: _:
        (inputs.crane.mkLib final).overrideToolchain (final.fenix.combine (lib.attrValues {
          inherit
            (final.fenix.stable)
            cargo
            clippy
            rust-analyzer
            rustc
            ;

          # Nightly rustfmt for the formatting options.
          inherit
            (final.fenix.default)
            rustfmt
            ;
        }));
    };
    packages =
      lib.mapAttrs (system: pkgs: {
        default = self.packages.${system}.dix;

        dix = pkgs.crane.buildPackage (pkgs.dix.cargoArguments
          // {
            inherit (pkgs.dix) cargoArtifacts;

            pname = "dix";
            cargoExtraArgs = "--package dix";

            doCheck = false;
          });
      })
      pkgsFor;

    devShells =
      lib.mapAttrs (system: pkgs: {
        default = self.devShells.${system}.dix;

        dix = pkgs.crane.devShell {
          packages = lib.attrValues {
            inherit
              (pkgs)
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
            export RUST_BACKTRACE="1"
          '';
        };
      })
      pkgsFor;

    checks =
      lib.mapAttrs (system: pkgs: let
        craneLib = inputs.crane.mkLib pkgs;
      in {
        inherit (self.packages.${system}) dix;

        dix-doctest = pkgs.crane.cargoDocTest (pkgs.dix.cargoArguments
          // {
            inherit (pkgs.dix) cargoArtifacts;
          });

        dix-nextest = pkgs.crane.cargoNextest (pkgs.dix.cargoArguments
          // {
            inherit (pkgs.dix) cargoArtifacts;
          });

        dix-clippy = pkgs.crane.cargoClippy (pkgs.dix.cargoArguments
          // {
            inherit (pkgs.dix) cargoArtifacts;

            env.CLIPPY_CONF_DIR = pkgs.writeTextDir "clippy.toml" (lib.readFile ./.clippy.toml);

            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

        dix-doc = pkgs.crane.cargoDoc (pkgs.dix.cargoArguments
          // {
            inherit (pkgs.dix) cargoArtifacts;
          });

        dix-fmt = pkgs.crane.cargoFmt {
          inherit (pkgs.dix) src;

          rustFmtExtraArgs = "--config-path ${./.rustfmt.toml}";
        };

        dix-toml-fmt = pkgs.crane.taploFmt {
          src = lib.sources.sourceFilesBySuffices pkgs.dix.src [".toml"];

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
      })
      pkgsFor;
  };
}
