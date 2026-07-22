# c4 — Claude Code Command Collector

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
install -m755 result/bin/c4 ~/.local/bin/c4
```

インストールせずに `nix run` で直接呼ぶこともできる（初回はビルドが走る。
hookは実行のたびにflake評価のオーバーヘッド（数百ms〜）を払うため、
気になる場合は上記のバイナリ配置を推奨）:

```json
{
  "type": "command",
  "command": "STORAGE_TYPE=csv nix run github:Xantibody/c4 --"
}
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
            "command": "STORAGE_TYPE=csv CSV_PATH=$HOME/.claude/c4.csv $HOME/.local/bin/c4"
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
            "command": "STORAGE_TYPE=csv CSV_PATH=$HOME/.claude/c4.csv $HOME/.local/bin/c4"
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
c4
```

## 収集されるレコード

```csv
timestamp,session_id,project,base_command,sub_command,flags,normalized_command,duration_ms,status
2026-07-22T03:04:36Z,sess-local,c4,git,commit,-m,git commit,49,success
2026-07-22T03:04:36Z,sess-local,c4,grep,,,grep,49,success
```

失敗したコマンドは `PostToolUseFailure` イベント経由で `status=failure` として
記録される（両イベントに同じhookを登録する）。
