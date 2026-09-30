# CI Pipeline

Run the full pipeline:

```sh
just ci
```

## Steps

| Step | Command | Gate |
|------|---------|------|
| Format | `just fmt-check` | Yes |
| Lint | `just lint` | Yes |
| Workspace library tests | `just test-unit` | Yes |
