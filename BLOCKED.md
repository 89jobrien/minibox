# Blocked remainder for issue #250

Remote registry search is intentionally bounded out of the initial Searchbox
contract.

The existing registry adapters support exact OCI image references for manifest
and blob operations. OCI Distribution does not define a portable repository
search endpoint, Docker Hub search uses a vendor API, and GHCR discovery requires
separate package API semantics and authentication policy. Adding one of those
paths would invent a cross-registry contract and credential flow that issue #250
does not specify.

Implemented now:

- deterministic local repository and tag search
- `mbx search <query> [--limit N]`
- protocol support with an explicit `remote` capability request
- a clear error for `mbx search --remote`

Unblock remote discovery by specifying supported registries, authentication,
pagination, result metadata, ranking merge rules, and failure behavior when local
and remote sources differ.
