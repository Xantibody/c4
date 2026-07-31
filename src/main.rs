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

Invoked by Claude Code's PostToolUse / PostToolUseFailure hooks.
Reads the hook JSON from stdin, then normalizes and persists the Bash
command. Not meant to be run interactively.

USAGE:
    echo '<hook JSON>' | c4        process a hook event and persist it
    c4 transcript                  scan Claude Code transcripts into conduct records
    c4 --persist                   (internal) read record JSON from stdin and store it
    c4 --help                      show this help

ENV:
    STORAGE_TYPE   r2 / csv / mock (default: csv)
    CSV_PATH       CSV output path (default: c4.csv)
    C4_DUMP        raw payload dump path (schema debugging; contains secrets verbatim)
    R2_BUCKET / R2_ENDPOINT / AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY

    C4_TRANSCRIPT_DIR    transcript root (default: ~/.claude/projects)
    CONDUCT_CSV_PATH     conduct CSV output (default: c4_conduct.csv)
    CONDUCT_STATE_PATH   incremental scan state (default: c4_conduct_state.json)
";

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--help" | "-h") => {
            print!("{USAGE}");
            return;
        }
        Some("transcript") => {
            if let Err(e) = collect_transcripts() {
                eprintln!("c4: transcript scan failed: {e:#}");
                std::process::exit(1);
            }
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

/// バッチモード: トランスクリプトを差分スキャンしてConductLogをCSVへ追記する。
/// hookと違いユーザー（またはcron）が直接叩くため、エラーは隠さず
/// 非0終了で返す。stateファイルが読み取り位置と未解決ペアを持つので
/// 何度実行しても重複レコードは生じない
fn collect_transcripts() -> anyhow::Result<()> {
    let dir = std::env::var("C4_TRANSCRIPT_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.claude/projects")
    });
    let csv_path = std::env::var("CONDUCT_CSV_PATH").unwrap_or_else(|_| "c4_conduct.csv".into());
    let state_path =
        std::env::var("CONDUCT_STATE_PATH").unwrap_or_else(|_| "c4_conduct_state.json".into());

    let mut state = match std::fs::read_to_string(&state_path) {
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| anyhow::anyhow!("corrupt state file {state_path}: {e}"))?,
        Err(_) => c4::transcript::ScanState::default(),
    };
    let records = c4::transcript::scan_dir(std::path::Path::new(&dir), &mut state)?;
    if !records.is_empty() {
        storage::csv_append(std::path::Path::new(&csv_path), &records)?;
    }
    std::fs::write(&state_path, serde_json::to_vec(&state)?)?;
    println!(
        "c4: appended {} conduct records to {csv_path} (scanned {dir})",
        records.len()
    );
    Ok(())
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
