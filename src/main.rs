use std::io::{IsTerminal, Read, Write};
use std::process::{Command, Stdio};

use c4::record::{NormalizedLog, build_records};
use c4::{hook, storage};
use time::OffsetDateTime;

/// hookとして呼ばれる親モード:
/// stdinのJSONをパース・正規化し、永続化は自分自身を `--persist` で
/// デタッチ起動した子プロセスに委ねて即終了する。Claude Codeは
/// hookプロセスの終了を待つため、ネットワークI/Oを親に置かない。
///
/// hookの失敗でClaude Code本体の作業を止めないため、エラーは
/// stderrに出して終了コード0で終える。
const USAGE: &str = "\
c4 — Claude Code Command Collector

Claude Code の PostToolUse / PostToolUseFailure hook から呼ばれ、
stdin の JSON を読んで Bash コマンドを正規化・保存する。
手動で対話実行するツールではない。

USAGE:
    echo '<hook JSON>' | c4        hookイベントを処理して保存
    c4 --persist                   (内部用) レコードJSONをstdinから読んで永続化
    c4 --help                      このヘルプを表示

ENV:
    STORAGE_TYPE   r2 / csv / mock (default: csv)
    CSV_PATH       CSV出力先 (default: c4.csv)
    C4_DUMP        生ペイロードの退避先 (スキーマ調査用・機密がそのまま残る)
    R2_BUCKET / R2_ENDPOINT / AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY
";

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--help" | "-h") => {
            print!("{USAGE}");
            return;
        }
        Some("--persist") => {
            reject_terminal_stdin();
            if let Err(e) = persist_from_stdin() {
                eprintln!("c4: persist failed: {e:#}");
            }
            return;
        }
        Some(other) => {
            eprintln!("c4: unknown argument: {other}\n");
            eprint!("{USAGE}");
            std::process::exit(2);
        }
        None => {}
    }
    reject_terminal_stdin();
    if let Err(e) = collect_and_spawn() {
        eprintln!("c4: {e:#}");
    }
}

/// 端末から直接叩かれた場合、stdinを待ってフリーズする代わりに
/// 使い方を出して終了する。hook経由（パイプ）では発動しない。
fn reject_terminal_stdin() {
    if std::io::stdin().is_terminal() {
        eprint!("{USAGE}");
        std::process::exit(2);
    }
}

fn collect_and_spawn() -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    dump_raw_payload(&input);
    let event = hook::parse(&input)?;
    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    let records = build_records(&event, OffsetDateTime::now_utc(), &hostname);
    if records.is_empty() {
        return Ok(());
    }
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--persist")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("stdin was requested as piped")
        .write_all(serde_json::to_vec(&records)?.as_slice())?;
    // waitしない: 子は孤児としてバックグラウンドで書き込みを続ける
    Ok(())
}

/// スキーマ調査用: C4_DUMP にパスが設定されていれば、
/// パース前の生ペイロードをJSONLで追記する。生コマンド（機密含む）が
/// そのまま残るデバッグ専用機能。失敗しても収集は続行する。
fn dump_raw_payload(input: &str) {
    let Ok(path) = std::env::var("C4_DUMP") else {
        return;
    };
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{}", input.replace('\n', " ")));
    if let Err(e) = result {
        eprintln!("c4: dump failed: {e}");
    }
}

/// 子モード: stdinからレコード配列を受け取りストレージへ保存する
fn persist_from_stdin() -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let records: Vec<NormalizedLog> = serde_json::from_str(&input)?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let storage = storage::from_env().await?;
            tokio::time::timeout(std::time::Duration::from_secs(30), storage.save(&records))
                .await
                .map_err(|_| anyhow::anyhow!("storage save timed out"))?
        })
}
