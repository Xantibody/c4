use std::io::{Read, Write};
use std::process::{Command, Stdio};

use claude_logger::record::{build_records, NormalizedLog};
use claude_logger::{hook, storage};
use time::OffsetDateTime;

/// hookとして呼ばれる親モード:
/// stdinのJSONをパース・正規化し、永続化は自分自身を `--persist` で
/// デタッチ起動した子プロセスに委ねて即終了する。Claude Codeは
/// hookプロセスの終了を待つため、ネットワークI/Oを親に置かない。
///
/// hookの失敗でClaude Code本体の作業を止めないため、エラーは
/// stderrに出して終了コード0で終える。
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--persist") {
        if let Err(e) = persist_from_stdin() {
            eprintln!("claude-logger: persist failed: {e:#}");
        }
        return;
    }
    if let Err(e) = collect_and_spawn() {
        eprintln!("claude-logger: {e:#}");
    }
}

fn collect_and_spawn() -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let event = hook::parse(&input)?;
    let records = build_records(&event, OffsetDateTime::now_utc());
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
