let
  moz_overlay = import (builtins.fetchTarball
    "https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz");
  nixpkgs = import <nixpkgs> { overlays = [ moz_overlay ]; };
in with nixpkgs;
stdenv.mkDerivation {
  name = "govdiff";
  buildInputs = [
    ((rustChannelOf { channel = "stable"; }).rust.override (old:
      { extensions = ["rust-src" "rust-analysis"]; }))
    rustfmt
    
    libiconv
    darwin.apple_sdk.frameworks.Security
    pkg-config
    openssl
  ];
  shellHook = ''
  '';
}