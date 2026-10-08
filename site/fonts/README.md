# Fonts

Self-hosted. No CDN request is made at render time, so the site works
offline and from `file://`. The `@font-face` rules and their system-stack
fallbacks are both in `../style.css`, so the site still renders if these
binaries go missing.

All faces are the **latin** subset, pulled from Fontsource. Every one is
under the SIL Open Font License 1.1.

## minibox

| File                                    | Family        | Weights          | Source                                       |
| --------------------------------------- | ------------- | ---------------- | -------------------------------------------- |
| `martian-mono-latin-wght-normal.woff2`  | martian-mono  | 100-900 variable | <https://fontsource.org/fonts/martian-mono>  |
| `commit-mono-latin-400-normal.woff2`    | commit-mono   | 400              | <https://fontsource.org/fonts/commit-mono>   |
| `commit-mono-latin-700-normal.woff2`    | commit-mono   | 700              | <https://fontsource.org/fonts/commit-mono>   |
| `ibm-plex-sans-latin-wght-normal.woff2` | ibm-plex-sans | 100-900 variable | <https://fontsource.org/fonts/ibm-plex-sans> |

Re-fetch:

```bash
base=https://cdn.jsdelivr.net/npm
curl -sLo assets/fonts/martian-mono-latin-wght-normal.woff2 \
  "$base/@fontsource-variable/martian-mono@latest/files/martian-mono-latin-wght-normal.woff2"
curl -sLo assets/fonts/commit-mono-latin-400-normal.woff2 \
  "$base/@fontsource/commit-mono@latest/files/commit-mono-latin-400-normal.woff2"
curl -sLo assets/fonts/commit-mono-latin-700-normal.woff2 \
  "$base/@fontsource/commit-mono@latest/files/commit-mono-latin-700-normal.woff2"
curl -sLo assets/fonts/ibm-plex-sans-latin-wght-normal.woff2 \
  "$base/@fontsource-variable/ibm-plex-sans@latest/files/ibm-plex-sans-latin-wght-normal.woff2"
```

Append a paragraph here explaining _why_ these faces suit this project.
The reasoning is what stops a later edit from swapping in a default.
