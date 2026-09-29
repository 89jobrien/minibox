# Configuration

## Repository Automation

Taskit workflow configuration lives in `taskit.toml` at the workspace root.
Its `[workspace].crates` list is the subset managed by Taskit, not the complete
17-member Cargo workspace inventory.
Daemon configuration is separate: minibox loads system and user TOML files,
then applies environment-variable overrides. See
[`README.md`](../../../README.md#configuration).

### Sections

| Section | Purpose |
|---------|---------|
| `[workspace]` | Crate list, propagation rules, offline skip |
| `[protocol]` | Contract surface drift detection |
| `[coverage]` | Reserved example; currently commented out |
| `[ci]` | Pipeline steps |
| `[flow]` | Git branching workflow |
