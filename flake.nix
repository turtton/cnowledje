{
  description = "Read-only Confluence CLI for Server/Data Center";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  inputs.systems.url = "github:nix-systems/default";
  inputs.flake-utils = {
    url = "github:numtide/flake-utils";
    inputs.systems.follows = "systems";
  };
  inputs.llm-agents.url = "github:numtide/llm-agents.nix";

  nixConfig = {
    extra-substituters = [ "https://cache.numtide.com" ];
    extra-trusted-public-keys = [ "niks3.numtide.com-1:DTx8wZduET09hRmMtKdQDxNNthLQETkc/yaX7M4qK0g=" ];
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      llm-agents,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ llm-agents.overlays.shared-nixpkgs ];
        };
      in
      {
        formatter = pkgs.nixfmt-tree;

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "cnowledje";
          version = "0.1.0";

          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.dbus ];

          meta = {
            description = "Read-only Confluence CLI for Server/Data Center";
            homepage = "https://github.com/turtton/cnowledje";
            license = pkgs.lib.licenses.mit;
            mainProgram = "cnowledje";
          };
        };

        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.bashInteractive
            pkgs.llm-agents.apm
          ];
        };
      }
    );
}
