# shell.nix for non-flakes nix-shell
{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    # Rust tools
    rustc
    cargo
    clippy
    rustfmt
  ] ++ (if pkgs.stdenv.isDarwin then [
    # macOS specific dependencies
    libiconv
  ] ++ (with pkgs.darwin.apple_sdk.frameworks; [
    Security
    SystemConfiguration
    CoreFoundation
  ])) else [
    # Linux specific dependencies
    openssl
    pkg-config
  ]);

  # Platform-specific shell hooks
  shellHook = 
    if pkgs.stdenv.isDarwin then ''
      # macOS specific settings
      export NIX_LDFLAGS="-L${pkgs.darwin.libiconv}/lib $NIX_LDFLAGS"
    '' else ''
      # Linux specific settings
      export PKG_CONFIG_PATH="${pkgs.openssl.dev}/lib/pkgconfig:$PKG_CONFIG_PATH"
      export LD_LIBRARY_PATH="${pkgs.openssl.out}/lib:$LD_LIBRARY_PATH"
    '';
}
