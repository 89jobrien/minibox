# Kubernetes-in-Docker via Sysbox

Source: coder/sysbox repo, docs/quickstart/kind.md

## How it works

Each Docker container acts as a K8s node, running under `sysbox-runc`
with user-namespace isolation. No `--privileged` needed. Containers are
connected via Docker networks to form a cluster.

## Basic workflow

```bash
# 1. Launch master node
docker run --runtime=sysbox-runc -d --name=k8s-master \
  nestybox/k8s-node:v1.18.2

# 2. Init K8s
docker exec k8s-master kubeadm init \
  --kubernetes-version=v1.18.2 \
  --pod-network-cidr=10.244.0.0/16

# 3. Copy kubeconfig to host
docker cp k8s-master:/etc/kubernetes/admin.conf $HOME/.kube/config

# 4. Install CNI (flannel)
kubectl apply -f kube-flannel.yml

# 5. Join worker nodes via kubeadm join token
```

## Networking

- Default bridge works but has limitations.
- User-defined bridge recommended for isolation:
  ```bash
  docker network create mynet
  docker run --runtime=sysbox-runc --net=mynet --name=k8s-master ...
  ```

## Storage: preloading pod images into node images

Two methods (same as DinD pattern):

1. **Docker build** — set `sysbox-runc` as default runtime, pull images
   during `RUN` step. Requires legacy builder (`DOCKER_BUILDKIT=0`).
2. **Docker commit** — run container, pull images inside, snapshot with
   `docker commit`.

Sysbox-EE reduces storage from ~10GB to ~1GB for a 10-node cluster
through shared backing stores.

## Security model

- Root inside the container maps to an unprivileged host UID
  (e.g. uid 165536).
- Sysbox-EE assigns exclusive user-namespace ID ranges per container,
  preventing cross-container UID collisions.

## Kindbox helper

Bash wrapper for cluster lifecycle:

```bash
kindbox create --num-workers=9 mycluster   # ~2 min
kindbox resize --num-workers=4 mycluster
kindbox destroy mycluster
```

## Relevance to minibox

- The "container as VM node" pattern is the same model minibox uses —
  each container is an isolated compute unit, not a microservice.
- User-namespace UID mapping (root -> unprivileged host UID) is
  directly relevant to minibox's security model.
- Preloading images into node images to speed cluster boot applies to
  any scenario where minibox containers need inner runtimes.
- The kindbox wrapper pattern (thin shell over docker commands) is
  analogous to minibox-cli wrapping miniboxd.
