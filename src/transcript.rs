use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::conduct::{ConductLog, normalize_tool_input};

/// トランスクリプトJSONLの1行。必要なフィールドだけ拾い、
/// 未知のフィールドは無視する。実ファイルのダンプで確認済みの形を正とする。
#[derive(Debug, Deserialize)]
struct Line {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    timestamp: String,
    #[serde(rename = "sessionId", default)]
    session_id: String,
    #[serde(default)]
    cwd: String,
    /// サブエージェント（subagents/agent-*.jsonl）の行はtrue
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
    /// hookペイロードと違い、トランスクリプトでは素の文字列
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    message: Option<Message>,
    /// 結果行にだけ載る実行結果。Bashではinterruptedフラグを持つ
    #[serde(rename = "toolUseResult", default)]
    tool_use_result: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Message {
    /// 文字列（プレーンな発話）と配列（tool_use / tool_result）の両形がある
    #[serde(default)]
    content: Value,
}

/// 1回のスキャン結果。pendingは結果行がまだ書かれていない
/// tool_useのドラフトで、次回スキャンに持ち越して解決する
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    pub records: Vec<ConductLog>,
    pub pending: Vec<ConductLog>,
    pub lines_seen: usize,
}

/// 行の並びからConductLogを組み立てる純粋関数。
/// tool_use（assistant行）とtool_result（user行）をtool_use_idで突き合わせ、
/// durationは両行のtimestamp差で近似する（permission待ちを含む）。
/// carriedには前回スキャンで未解決だったドラフトを渡す。
pub fn scan<I>(lines: I, carried: Vec<ConductLog>) -> ScanOutcome
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut pending: HashMap<String, ConductLog> = carried
        .into_iter()
        .map(|r| (r.tool_use_id.clone(), r))
        .collect();
    let mut records = Vec::new();
    let mut lines_seen = 0;
    for line in lines {
        lines_seen += 1;
        let Ok(line) = serde_json::from_str::<Line>(line.as_ref()) else {
            continue;
        };
        let Some(content) = line.message.as_ref().map(|m| &m.content) else {
            continue;
        };
        let Some(items) = content.as_array() else {
            continue;
        };
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("tool_use") if line.kind == "assistant" => {
                    if let Some(draft) = draft_from_tool_use(&line, item) {
                        pending.insert(draft.tool_use_id.clone(), draft);
                    }
                }
                Some("tool_result") if line.kind == "user" => {
                    let Some(id) = item.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    if let Some(mut draft) = pending.remove(id) {
                        draft.duration_ms = duration_between(&draft.timestamp, &line.timestamp);
                        draft.status = resolve_status(item, line.tool_use_result.as_ref());
                        records.push(draft);
                    }
                }
                _ => {}
            }
        }
    }
    let mut pending: Vec<ConductLog> = pending.into_values().collect();
    pending.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    ScanOutcome {
        records,
        pending,
        lines_seen,
    }
}

/// assistant行のtool_useから、結果待ちのドラフトレコードを作る
fn draft_from_tool_use(line: &Line, item: &Value) -> Option<ConductLog> {
    let id = item.get("id").and_then(Value::as_str)?;
    let name = item.get("name").and_then(Value::as_str)?;
    let empty = Value::Null;
    let input = item.get("input").unwrap_or(&empty);
    let summary = normalize_tool_input(name, input, &line.session_id);
    Some(ConductLog {
        timestamp: line.timestamp.clone(),
        session_id: line.session_id.clone(),
        tool_use_id: id.to_string(),
        project: std::path::Path::new(&line.cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        source: if line.is_sidechain {
            "sidechain".to_string()
        } else {
            "main".to_string()
        },
        tool_name: name.to_string(),
        detail: summary.detail,
        path_hash: summary.path_hash,
        path_kind: summary.path_kind,
        duration_ms: None,
        status: String::new(),
        effort: line.effort.clone().unwrap_or_default(),
    })
}

/// 中断はユーザーによる行動修正のシグナルなのでfailureと区別して残す
fn resolve_status(result_item: &Value, line_result: Option<&Value>) -> String {
    let interrupted = line_result
        .and_then(|r| r.get("interrupted"))
        .and_then(Value::as_bool)
        == Some(true);
    if interrupted {
        "interrupted".to_string()
    } else if result_item.get("is_error").and_then(Value::as_bool) == Some(true) {
        "failure".to_string()
    } else {
        "success".to_string()
    }
}

/// RFC3339同士の差をmsで返す。パース不能・負の差はNone
fn duration_between(start: &str, end: &str) -> Option<u64> {
    let start = OffsetDateTime::parse(start, &Rfc3339).ok()?;
    let end = OffsetDateTime::parse(end, &Rfc3339).ok()?;
    u64::try_from((end - start).whole_milliseconds()).ok()
}

/// ファイルごとの読み取り位置と未解決ドラフト。JSONで永続化して
/// 追記型の差分スキャンを冪等にする
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanState {
    #[serde(default)]
    pub files: HashMap<String, FileState>,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileState {
    /// 処理済み行数。次回はこの行数だけ読み飛ばす
    pub lines: usize,
    /// 結果行がまだ現れていないtool_useドラフト
    pub pending: Vec<ConductLog>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn use_line(ts: &str, id: &str, name: &str, input: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","sessionId":"sess-1","cwd":"/Users/me/Repository/c4","isSidechain":false,"effort":"high","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{input}}}]}}}}"#
        )
    }

    fn result_line(ts: &str, id: &str, extra_item: &str, line_result: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"{ts}","sessionId":"sess-1","cwd":"/Users/me/Repository/c4","message":{{"content":[{{"type":"tool_result","tool_use_id":"{id}"{extra_item}}}]}},"toolUseResult":{line_result}}}"#
        )
    }

    #[test]
    fn pairs_tool_use_with_result_and_derives_duration() {
        let lines = [
            use_line(
                "2026-07-24T15:38:14.000Z",
                "toolu_1",
                "Read",
                r#"{"file_path":"/repo/src/main.rs","limit":50}"#,
            ),
            result_line(
                "2026-07-24T15:38:14.719Z",
                "toolu_1",
                "",
                r#"{"stdout":"...","interrupted":false}"#,
            ),
        ];
        let outcome = scan(&lines, vec![]);
        assert_eq!(outcome.pending, vec![]);
        assert_eq!(outcome.lines_seen, 2);
        let r = &outcome.records[0];
        assert_eq!(r.tool_name, "Read");
        assert_eq!(r.project, "c4");
        assert_eq!(r.source, "main");
        assert_eq!(r.detail, "limit");
        assert_eq!(r.path_kind, "rs");
        assert_eq!(r.path_hash.len(), 16);
        assert_eq!(r.duration_ms, Some(719));
        assert_eq!(r.status, "success");
        assert_eq!(r.effort, "high");
    }

    #[test]
    fn is_error_result_is_a_failure() {
        let lines = [
            use_line("2026-07-24T15:00:00Z", "toolu_2", "Bash", r#"{"command":"cargo test"}"#),
            result_line("2026-07-24T15:00:03Z", "toolu_2", r#","is_error":true"#, r#""Exit code 1""#),
        ];
        let outcome = scan(&lines, vec![]);
        assert_eq!(outcome.records[0].status, "failure");
        assert_eq!(outcome.records[0].detail, "cargo test");
    }

    #[test]
    fn interrupted_result_is_distinguished_from_failure() {
        let lines = [
            use_line("2026-07-24T15:00:00Z", "toolu_3", "Bash", r#"{"command":"sleep 100"}"#),
            result_line(
                "2026-07-24T15:00:09Z",
                "toolu_3",
                r#","is_error":true"#,
                r#"{"stdout":"","interrupted":true}"#,
            ),
        ];
        assert_eq!(scan(&lines, vec![]).records[0].status, "interrupted");
    }

    #[test]
    fn unmatched_use_is_carried_as_pending_and_resolved_next_scan() {
        let first = scan(
            [use_line("2026-07-24T15:00:00Z", "toolu_4", "Write", r#"{"file_path":"/a/b.rs"}"#)],
            vec![],
        );
        assert_eq!(first.records, vec![]);
        assert_eq!(first.pending.len(), 1);

        let second = scan(
            [result_line("2026-07-24T15:00:01Z", "toolu_4", "", "{}")],
            first.pending,
        );
        assert_eq!(second.pending, vec![]);
        assert_eq!(second.records[0].tool_name, "Write");
        assert_eq!(second.records[0].duration_ms, Some(1000));
    }

    #[test]
    fn sidechain_lines_are_marked_as_such() {
        let line = use_line("2026-07-24T15:00:00Z", "toolu_5", "Grep", r#"{"pattern":"x"}"#)
            .replace(r#""isSidechain":false"#, r#""isSidechain":true"#);
        assert_eq!(scan([line], vec![]).pending[0].source, "sidechain");
    }

    #[test]
    fn malformed_and_plain_text_lines_are_counted_but_skipped() {
        let lines = [
            "not json".to_string(),
            r#"{"type":"user","message":{"content":"just a prompt"}}"#.to_string(),
            r#"{"type":"summary","summary":"compacted"}"#.to_string(),
        ];
        let outcome = scan(&lines, vec![]);
        assert_eq!(outcome, ScanOutcome { records: vec![], pending: vec![], lines_seen: 3 });
    }

    #[test]
    fn scan_state_round_trips_through_json() {
        let mut state = ScanState::default();
        state.files.insert(
            "a.jsonl".to_string(),
            FileState { lines: 42, pending: vec![] },
        );
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<ScanState>(&json).unwrap(), state);
    }
}
