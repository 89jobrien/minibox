# SOPS-backed rustic secret workflow.

set shell := ["bash", "-cu"]

# Materialize the rustic repository password from SOPS into RUSTIC_PASSWORD_FILE.
rustic-password-materialize:
    @set -euo pipefail; \
    secret_file="{{justfile_directory()}}/secrets/rustic.sops.yaml"; \
    password_file="${RUSTIC_PASSWORD_FILE:-$HOME/.config/rustic/password}"; \
    key_file="${SOPS_AGE_KEY_FILE:-$HOME/.config/sops/age/keys.txt}"; \
    if [ ! -f "$secret_file" ]; then \
        echo "error: missing encrypted secret: $secret_file" >&2; \
        exit 1; \
    fi; \
    if [ ! -f "$key_file" ]; then \
        echo "error: missing SOPS age key: $key_file" >&2; \
        exit 1; \
    fi; \
    mkdir -p "$(dirname "$password_file")"; \
    chmod 700 "$(dirname "$password_file")"; \
    tmp="$(mktemp)"; \
    SOPS_AGE_KEY_FILE="$key_file" sops --decrypt --extract '["rustic_password"]' "$secret_file" > "$tmp"; \
    chmod 600 "$tmp"; \
    mv "$tmp" "$password_file"; \
    chmod 600 "$password_file"; \
    echo "[OK] materialized rustic password file: $password_file"

# Initialize the configured rustic repository.
rustic-init: rustic-password-materialize
    @RUSTIC_REPOSITORY="${RUSTIC_REPOSITORY:-$HOME/.local/share/rustic/minibox}" \
        RUSTIC_PASSWORD_FILE="${RUSTIC_PASSWORD_FILE:-$HOME/.config/rustic/password}" \
        rustic init

# Verify rustic can read the configured repository.
rustic-verify: rustic-password-materialize
    @RUSTIC_REPOSITORY="${RUSTIC_REPOSITORY:-$HOME/.local/share/rustic/minibox}" \
        RUSTIC_PASSWORD_FILE="${RUSTIC_PASSWORD_FILE:-$HOME/.config/rustic/password}" \
        rustic snapshots
