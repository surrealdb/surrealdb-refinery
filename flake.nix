{
  description = "SurrealMMP API";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05-small";
    # Fixed version for k3d 5.8.2
    pkgs-02032da.url = "github:NixOS/nixpkgs/02032da4af073d0f6110540c8677f16d4be0117f";
    flake-utils.url = "github:numtide/flake-utils/v1.0.0";
    fenix.url = "github:nix-community/fenix";
  };

  outputs = { self, nixpkgs, pkgs-02032da, flake-utils, fenix }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = nixpkgs.legacyPackages.${system};
      pkgs-02032 = pkgs-02032da.legacyPackages.${system};
      rust-toolchain = fenix.packages.${system}.stable.withComponents [
        "cargo"
        "rust-std"
        "clippy"
        "rustc"
        "rustfmt"
        "rust-src"
      ];
    in
    with pkgs;
      {
        devShells.default = mkShell {
          packages = [
            awscli2
            kubectl
            kustomize
            pkgs-02032.k3d
            kubectx
            kubernetes-helm
            envsubst
            curl
            rust-toolchain
            granted
          ] ++ (if stdenv.isDarwin then [
            # macOS specific dependencies
            libiconv
            darwin.apple_sdk.frameworks.Security
            darwin.apple_sdk.frameworks.SystemConfiguration
            darwin.apple_sdk.frameworks.CoreFoundation
          ] else [
            # Linux specific dependencies
            openssl
            pkg-config
          ]);
          # Add platform-specific environment settings
          NIX_LDFLAGS = lib.optionalString stdenv.isDarwin "-L${darwin.libiconv}/lib";
          # Add OpenSSL configuration for Linux
          shellHook = lib.optionalString (!stdenv.isDarwin) ''
            export PKG_CONFIG_PATH="${pkgs.openssl.dev}/lib/pkgconfig:$PKG_CONFIG_PATH"
            export LD_LIBRARY_PATH="${pkgs.openssl.out}/lib:$LD_LIBRARY_PATH"
          '';
          RUST_SRC_PATH = "${rust-toolchain}/lib/rustlib/src/rust/library";
        };
      }
  );
}
