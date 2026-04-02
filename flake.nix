{
  description = "Kanade — music server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };

        # Runtime / build-time native deps
        nativeBuildInputs = with pkgs; [
          rustToolchain
          pkg-config
        ];

        # System libraries needed at runtime
        buildInputs = with pkgs; [
          openssl
        ];

        # Runtime tools
        packages = with pkgs; [
          mpd
          mpc
          tmux
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          inherit buildInputs nativeBuildInputs packages;

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          shellHook = ''
            echo "🦀 Kanade dev shell"
            echo "  rustc  $(rustc --version)"
            echo "  cargo  $(cargo --version)"
          '';
        };
      }
    );
}
