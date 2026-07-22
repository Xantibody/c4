# c4 — Claude Code Command Collector

A CLI tool that collects Bash commands executed by Claude Code via the
PostToolUse hook, normalizes them, and persists them to Cloudflare R2 or
a local CSV file.

Command arguments, file paths, and messages are stripped before persisting,
so secrets never reach the storage layer. See [docs/design.md](docs/design.md)
for the design details (Japanese).

## Build and development

```sh
# Enter the devShell (automatic with direnv)
nix develop

# Run all checks (clippy + fmt + test)
just check

# E2E check in CSV mode
just smoke

# Release build
nix build
```

## Installation and Claude Code integration

Consume it as a flake input (e.g. via home-manager) or install it into
your profile:

```sh
# nix profile
nix profile install github:Xantibody/c4

# home-manager: add as a flake input and expose via an overlay
#   inputs.c4.url = "github:Xantibody/c4";
#   (final: _: { c4 = inputs.c4.packages.${final.system}.default; })
```

Once it is on your PATH, the hook can invoke it as plain `c4`.

You can also run it directly with `nix run` without installing (the first
run triggers a build; every hook invocation then pays flake evaluation
overhead of a few hundred ms, so installation is recommended):

```json
{
  "type": "command",
  "command": "STORAGE_TYPE=csv nix run github:Xantibody/c4 --"
}
```

Register the hook in `~/.claude/settings.json`
(see [examples/settings.json](examples/settings.json)):

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "STORAGE_TYPE=csv CSV_PATH=$HOME/.claude/c4.csv c4"
          }
        ]
      }
    ],
    "PostToolUseFailure": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "STORAGE_TYPE=csv CSV_PATH=$HOME/.claude/c4.csv c4"
          }
        ]
      }
    ]
  }
}
```

To persist to R2 instead, switch the environment variables:

```sh
STORAGE_TYPE=r2 \
R2_BUCKET=my-bucket \
R2_ENDPOINT=https://<account-id>.r2.cloudflarestorage.com \
AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... \
c4
```

## Collected records

```csv
timestamp,session_id,tool_use_id,project,segment_index,connector,base_command,sub_command,flags,normalized_command,duration_ms,status
2026-07-22T03:04:36Z,sess-local,toolu_x,c4,0,,git,commit,-m,git commit,49,success
2026-07-22T03:04:36Z,sess-local,toolu_x,c4,1,&&,cat,,,cat,49,success
2026-07-22T03:04:36Z,sess-local,toolu_x,c4,2,|,grep,,,grep,49,success
```

`tool_use_id` + `segment_index` + `connector` let you reconstruct compound
command chains at analysis time (used to detect replacement candidates such
as `cat | grep` → `rg`).

Failed commands are recorded with `status=failure` via the
`PostToolUseFailure` event (register the same hook on both events).
