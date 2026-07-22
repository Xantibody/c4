# c4 (Claude Code Command Collector) 設計文書

Claude Code の Hook 機能を使い、作業中に実行された Bash コマンドを収集・正規化して
永続化（Cloudflare R2 / ローカルCSV）する Rust 製 CLI ツール。

## 1. 全体フロー

```text
[Claude Code]
   │  PostToolUse hook (matcher: Bash)
   ▼  stdin / JSON
[c4 親プロセス]          … パース + 正規化のみ。即終了
   │  自分自身を --persist でデタッチ起動し、レコードJSONをstdinへ
   ▼
[c4 --persist 子プロセス] … バックグラウンドで永続化
   │
   ▼
[Cloudflare R2 (JSONL)] / [ローカルCSV]
```

### なぜ2プロセスに分けるか

Claude Code は hook プロセスの終了を待ってから次の動作に進む。ネットワークI/O
（R2へのPUT）を hook プロセス内で行うと、遅延や障害がそのまま Claude Code の
体感速度に跳ね返る。そこで:

- **親**: stdin の読み取り → JSONパース → 正規化 → 子プロセス起動、で即終了する。
  CPUのみの処理でミリ秒オーダー。
- **子**: 孤児プロセスとしてストレージへの書き込みを行う。30秒でタイムアウト。

また hook の失敗が Claude Code の作業を妨げないよう、親は
いかなるエラーでも stderr に出力するだけで**終了コード0**で終える。

## 2. 入力データ仕様 (stdin)

Claude Code の PostToolUse hook が渡す実際のJSON形式を正とする:

```json
{
  "session_id": "sess-xxxx-xxxx",
  "transcript_path": "...",
  "cwd": "...",
  "hook_event_name": "PostToolUse",
  "tool_name": "Bash",
  "tool_input": { "command": "git commit -m 'feat: secret string'" },
  "tool_response": { "...": "..." }
}
```

簡略形式（`hook` / `tool` / `input` キー）も serde alias で受け付ける。
未知のフィールドは無視する。`tool_name` が `Bash` 以外のイベントは収集しない。

## 3. 正規化・出力データ構造

CSV / DuckDB / SQLite での集計を見据え、1行1レコードのフラット構造とする。

| 項目                 | 内容                                        | 例                     |
| -------------------- | ------------------------------------------- | ---------------------- |
| `timestamp`          | 実行日時 (ISO8601 / RFC3339, UTC)           | `2026-07-22T03:00:00Z` |
| `session_id`         | Claude Code セッションID                    | `sess-xxxx`            |
| `base_command`       | メインコマンド                              | `git`                  |
| `sub_command`        | サブコマンド（無ければ空文字）              | `commit`               |
| `flags`              | スペース区切りのフラグ名（値は含まない）    | `--amend -m`           |
| `normalized_command` | 集計キーとなる整形済みコマンド              | `git commit`           |

### 正規化ルール

1. コマンド文字列を `shell-words` でシェル的にトークン分割する
   （クォートを尊重するため、`grep 'a|b'` の `|` で誤分割しない）。
2. パイプ `|`・論理演算子 `&&` `||`・逐次実行 `;` で複合コマンドを分割し、
   **セグメントごとに1レコード**を生成する。
3. セグメント先頭の `FOO=bar` 形式の環境変数代入は読み飛ばす（機密を含みうるため）。
4. 先頭トークンを `base_command` とする。
5. サブコマンドを持つ主要CLI（`git` `npm` `pnpm` `yarn` `docker` `cargo` `aws`
   `kubectl` `gh` `go` `nix` `just`）で、第2トークンがフラグ（`-`始まり）で
   なければそれを `sub_command` とする。
6. フラグは**名前のみ**をソート・重複排除して `flags` に残す。値は残さない:
   - `--name=value` → `=` の手前まで（`--password=secret` → `--password`）
   - 短フラグは英字のみ3文字以内（`-m` `-la` `-rf`）ならそのまま。
     くっつき値の可能性がある長いもの（`-psecret123`）は先頭1文字（`-p`）に
     切り詰め、英字で始まらないもの（`head -5` の `-5`）は値とみなして捨てる
   - 裸の `--` 以降はオペランドなので収集しない
7. **メッセージ・ファイルパス・引数・環境変数値などはすべて切り捨てる**。
   機密情報は永続化層に到達しない。
8. クォート不整合などパース不能な入力はレコードを生成しない（安全側に倒す）。

### 正規化例

| 入力 | base | sub | flags | normalized |
| ---- | ---- | --- | ----- | ---------- |
| `git commit -m "fix typo"`          | `git` | `commit` | `-m`     | `git commit` |
| `npm run dev --port 3000`           | `npm` | `run`    | `--port` | `npm run`    |
| `ls -la /path/to/dir`               | `ls`  | ``       | `-la`    | `ls`         |
| `cat a.txt \| grep x && git status` | 3レコード: `cat` / `grep` / `git status` | | | |

## 4. モジュール構成

```text
src/
├── main.rs          # 親/子モードの分岐・プロセス制御（薄いI/O層）
├── lib.rs
├── hook.rs          # stdin JSONのパース (HookEvent)
├── normalize.rs     # コマンド正規化（純粋関数・単体テストの主戦場）
├── record.rs        # NormalizedLog の組み立て（純粋関数）
└── storage/
    ├── mod.rs       # Storageトレイト + STORAGE_TYPEによるファクトリ
    ├── csv.rs       # ローカルCSVへ追記
    ├── mock.rs      # テスト用インメモリ実装
    └── r2.rs        # Cloudflare R2 (S3互換) へPUT
```

設計原則: **データ整形（純粋関数）とストレージ操作（I/O）を分離**する。
`normalize` / `build_records` は入出力が値のみの純粋関数で、ネットワークや
ファイルシステムに触れないため高速に単体テストできる。

```rust
#[async_trait]
pub trait Storage {
    async fn save(&self, logs: &[NormalizedLog]) -> anyhow::Result<()>;
}
```

`save` がスライスを取るのは、1回の hook 呼び出し（複合コマンド）から複数
レコードが生じるため。まとめて1回の書き込みにする。

### R2 の Append-only 戦略

S3系ストレージは追記ができない。hook 呼び出しごとに日付パーティション付き
キーで JSONL オブジェクトを1つ PUT する:

```text
logs/dt=2026-07-22/<epoch-nanos>-<session_id>.jsonl
```

DuckDB からは `read_json_auto('s3://bucket/logs/dt=*/*.jsonl')` のように
まとめて読める。小さいオブジェクトが多数できるが、書き込み側の単純さと
ロック不要のAppend-onlyを優先する。集約が必要になったら日次バッチで
コンパクションする（将来課題）。

## 5. 設定（環境変数）

| 変数                                          | 意味                                   | デフォルト          |
| --------------------------------------------- | -------------------------------------- | ------------------- |
| `STORAGE_TYPE`                                | `r2` / `csv` / `mock`                  | `csv`               |
| `CSV_PATH`                                    | CSV出力先パス                          | `c4.csv` |
| `R2_BUCKET`                                   | R2バケット名 (r2時必須)                | -                   |
| `R2_ENDPOINT`                                 | R2のS3互換エンドポイント (r2時必須)    | -                   |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | R2認証情報 (aws-sdkが標準参照)         | -                   |

## 6. Claude Code 連携設定

`~/.claude/settings.json`（実際の Claude Code hooks スキーマに準拠）:

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
    ]
  }
}
```

サンプルは [`examples/settings.json`](../examples/settings.json) に置いている。

## 7. 開発環境と CI/CD

- ツールチェーンは `flake.nix`（`rust-overlay` の stable 最新 + `flake.lock` で固定）
  から取得し、ローカルと CI が常に同一の rustc / clippy / rustfmt を使う。
- `direnv` (`.envrc: use flake`) で devShell に自動入場。
- タスクランナーは `just`（`just check` = lint + format-check + test）。
- フォーマッタは treefmt（nixfmt / rustfmt / taplo）。`nix fmt` で一括整形。
- GitHub Actions (`.github/workflows/ci.yml`) は fmt / clippy / test / nix build
  の4ジョブを `nix develop --command` 経由で実行する。
- 配布は `nix build`（`packages.default`）。hook 用には
  `nix profile install .` またはビルド成果物を `~/.local/bin` に配置する。

## 8. テスト戦略

- 正規化 (`normalize.rs`)・レコード組み立て (`record.rs`) は純粋関数として
  0-1-N + 境界ケース（クォート、フラグ、区切り文字のみ、パース不能）を網羅。
- ストレージは `MockStorage`（インメモリ）と `CsvStorage`（tempdir実書き込み）
  を単体テスト。`R2Storage` はキー生成のみ単体テストし、PUT自体は実環境で確認。
- E2E は `just smoke`（CSVモードで hook JSON を流し込み、出力を確認）。
