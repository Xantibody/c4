# Run clippy for linting
lint:
    cargo clippy --all-targets -- -D warnings

# Run formatter
format:
    cargo fmt

# Run formatter check (CI用)
format-check:
    cargo fmt --check

# Run tests
test:
    cargo test

# Run all checks (lint + format-check + test)
check: lint format-check test

# hookに渡されるJSONを模したローカル動作確認
smoke:
    echo '{"hook_event_name":"PostToolUse","tool_name":"Bash","session_id":"sess-local","tool_input":{"command":"git commit -m secret && cat foo | grep bar"}}' | STORAGE_TYPE=csv cargo run --quiet
    sleep 0.5
    cat c4.csv
