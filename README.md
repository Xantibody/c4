# claude-logger

Claude Code の PostToolUse hook から Bash コマンドを収集・正規化して
Cloudflare R2 / ローカルCSV に永続化する CLI ツール。

コマンドの引数・パス・メッセージは保存前に切り捨てられるため、
機密情報は永続化層に到達しない。設計の詳細は [docs/design.md](docs/design.md) を参照。

## ビルドと開発

```sh
# devShellに入る (direnvなら自動)
nix develop

# 全チェック (clippy + fmt + test)
just check

# CSVモードでのE2E動作確認
just smoke

# リリースビルド
nix build
```

## インストールと Claude Code 連携

```sh
nix build
install -m755 result/bin/claude-logger ~/.local/bin/claude-logger
```

`~/.claude/settings.json` に hook を登録する（[examples/settings.json](examples/settings.json)）:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "STORAGE_TYPE=csv CSV_PATH=$HOME/.claude/claude-logger.csv $HOME/.local/bin/claude-logger"
          }
        ]
      }
    ]
  }
}
```

R2 に保存する場合は環境変数を切り替える:

```sh
STORAGE_TYPE=r2 \
R2_BUCKET=my-bucket \
R2_ENDPOINT=https://<account-id>.r2.cloudflarestorage.com \
AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... \
claude-logger
```

## 収集されるレコード

```csv
timestamp,session_id,base_command,sub_command,normalized_command
2026-07-22T03:04:36Z,sess-local,git,commit,git commit
2026-07-22T03:04:36Z,sess-local,grep,,grep
```
