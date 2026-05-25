#!/bin/bash
# Default behavior for surfpool.
# Override by passing arguments to `docker run`.
# Note: when passing custom args, ensure `--no-tui` is included.
set -euo pipefail

if [ $# -eq 0 ]; then
    echo "Starting surfpool with default configuration..."
    exec surfpool start --no-tui
else
    echo "Starting surfpool with custom arguments: $*"
    exec surfpool "$@"
fi
