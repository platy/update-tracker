let
  moz_overlay = import (builtins.fetchTarball
    "https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz");
  nixpkgs = import <nixpkgs> { overlays = [ moz_overlay ]; };
in with nixpkgs;
stdenv.mkDerivation {
  name = "govdiff";
  buildInputs = [
    # generic rust
    ((rustChannelOf { channel = "stable"; }).rust.override (old:
      { extensions = ["rust-src" "rust-analysis"]; }))
    rustfmt
    libiconv

    # for project dependencies
    darwin.apple_sdk.frameworks.Security
    pkg-config
    openssl

    # for container build
    podman
    xz
    gvproxy
    qemu

    # for k8s mgmt
    kubectl
  ];
  shellHook = ''
  export KUBECONFIG=$HOME/kubeconfig-k3.yaml
  # export CONTAINERS_HELPER_BINARY_DIR=${gvproxy}/bin
  alias k8=kubectl
  alias prod-logs="kubectl logs -lapp=update-tracker -c update-tracker"
  alias prod-email-logs="kubectl logs -lapp=update-tracker -c smtp-dump"
  alias podman-start="CONTAINERS_CONF=containers.conf podman machine start"
  '';
}

# podman uses qemu for its `podman machine` virtual machine for doing containers on non-linux machines and gvproxy for the networking. Somehow it finds qemu correctly when installed in nix (possibly because it checks the path) but it doesn't find gvproxy as it looks in some default system locations (https://github.com/containers/common/blob/2d46695412078739031f92901d73e28f01200d3a/pkg/config/config_darwin.go#L19-L30) and not on the path
# one solution is to create a conf file which points to the gvproxy dir and then tell podman to use that conf file. But that doesn't seem good as it will override the user's config in their dotfiles
# better seems to be to set CONTAINERS_HELPER_BINARY_DIR, which they say is used in testing https://github.com/containers/common/blob/e7f2cb74164f2d9bf3d2f65613cf95bb6eaba6a2/pkg/config/config.go#L1171-L1172
# but maybe that was added to common recently and isn't vendored in yet
