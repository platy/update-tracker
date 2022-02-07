set -e

# deploy-patch
# check git clean
# if [[ -n "$(git status --porcelain 2> /dev/null)" ]]; then
#     echo "git not clean"
#     exit 1
# fi
# cargo test, clippy, next
cargo fmt --check
cargo test
cargo clippy
# update patch version
VERSION=$(sed --quiet 's/^version = "\(.*\)"/\1/p' server/Cargo.toml)
TAG="rg.nl-ams.scw.cloud/njkonl/update-tracker:$VERSION"
# make sure that this tag hasn't already been built
if $(podman image exists $TAG); then echo "Tag for $VERSION already built"; exit 1; fi
# update k8 config
sed -i "s|rg.nl-ams.scw.cloud/njkonl/update-tracker:\(.*\)$|$TAG|" deploy.yaml
git add deploy.yaml
# docker build & tag
CONTAINERS_CONF=containers.conf podman machine start || true
podman build -t $TAG .

# git commit
git commit -m "Deploy patch version $VERSION"
# docker push
podman push $TAG
# k8 apply
kubectl apply -f deploy.yaml
# wait for rollout
kubectl rollout status deployment/update-tracker
# tail logs
kubectl logs -lapp=update-tracker -c update-tracker -f
