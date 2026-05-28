{
  description = "minibox — container runtime";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, crane, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Common args for crane builds
        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          pname = "minibox";
          strictDeps = true;

          nativeBuildInputs = with pkgs; [
            pkg-config
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            pkgs.apple-sdk_15
            (pkgs.darwinMinVersionHook "10.12")
          ];

          buildInputs = with pkgs; [
            openssl
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
            pkgs.libseccomp
          ];
        };

        # Build deps first (cached layer)
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        minibox = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          # Build both binaries
          cargoExtraArgs = "-p miniboxd -p mbx";
        });

      in {
        checks = {
          inherit minibox;

          minibox-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "-- -D warnings";
          });

          minibox-fmt = craneLib.cargoFmt {
            inherit src;
          };
        };

        packages = {
          default = minibox;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = with pkgs; [
            just
            cargo-nextest
            cargo-deny
            cargo-audit
          ];
        };
      }
    );
}
