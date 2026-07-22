use std::path::Path;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::hook::HookEvent;
use crate::normalize::normalize;

/// 永続化する1行1レコードのフラット構造。
/// CSV / DuckDB / SQLite での集計を想定し、ネストさせない。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedLog {
    /// 実行日時 (ISO8601 UTC)
    pub timestamp: String,
    pub session_id: String,
    /// 1回のBashツール呼び出しのグループキー。複合コマンドの
    /// チェーン復元とduration_msの重複排除に使う
    pub tool_use_id: String,
    /// cwdのbasename。フルパスは個人情報を含みうるため残さない
    pub project: String,
    /// 複合コマンド内の位置（0始まり）
    pub segment_index: u32,
    /// このセグメントの直前の演算子 ("" / "|" / "&&" / "||" / ";")
    pub connector: String,
    pub base_command: String,
    pub sub_command: String,
    /// スペース区切りのフラグ名 (例: "--amend -m")。値は含まない
    pub flags: String,
    pub normalized_command: String,
    /// Bashツール全体の実行時間。複合コマンドでは各レコードに同値が入る
    pub duration_ms: Option<u64>,
    /// success / failure (PostToolUseFailure発火時)
    pub status: String,
}

/// HookEventから保存対象レコードを組み立てる純粋関数。
/// Bash以外のツールやコマンドが正規化不能な場合は空を返す。
pub fn build_records(event: &HookEvent, timestamp: OffsetDateTime) -> Vec<NormalizedLog> {
    let Some(command) = event.bash_command() else {
        return vec![];
    };
    let timestamp = timestamp
        .format(&Rfc3339)
        .expect("RFC3339 formatting cannot fail for a valid OffsetDateTime");
    let project = Path::new(&event.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    normalize(command)
        .into_iter()
        .map(|c| NormalizedLog {
            timestamp: timestamp.clone(),
            session_id: event.session_id.clone(),
            tool_use_id: event.tool_use_id.clone(),
            project: project.clone(),
            segment_index: c.segment_index,
            connector: c.connector,
            base_command: c.base_command,
            sub_command: c.sub_command,
            flags: c.flags.join(" "),
            normalized_command: c.normalized,
            duration_ms: event.duration_ms,
            status: event.status().to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn bash_event(command: &str) -> HookEvent {
        crate::hook::parse(&format!(
            r#"{{
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "session_id": "sess-test",
                "tool_use_id": "toolu_abc",
                "cwd": "/Users/me/Repository/c4",
                "duration_ms": 49,
                "tool_input": {{"command": {}}}
            }}"#,
            serde_json::to_string(command).unwrap()
        ))
        .unwrap()
    }

    #[test]
    fn builds_flat_records_with_execution_context() {
        let records = build_records(
            &bash_event("git commit -m secret && ls"),
            datetime!(2026-07-22 03:00:00 UTC),
        );
        assert_eq!(
            records,
            vec![
                NormalizedLog {
                    timestamp: "2026-07-22T03:00:00Z".to_string(),
                    session_id: "sess-test".to_string(),
                    tool_use_id: "toolu_abc".to_string(),
                    project: "c4".to_string(),
                    segment_index: 0,
                    connector: "".to_string(),
                    base_command: "git".to_string(),
                    sub_command: "commit".to_string(),
                    flags: "-m".to_string(),
                    normalized_command: "git commit".to_string(),
                    duration_ms: Some(49),
                    status: "success".to_string(),
                },
                NormalizedLog {
                    timestamp: "2026-07-22T03:00:00Z".to_string(),
                    session_id: "sess-test".to_string(),
                    tool_use_id: "toolu_abc".to_string(),
                    project: "c4".to_string(),
                    segment_index: 1,
                    connector: "&&".to_string(),
                    base_command: "ls".to_string(),
                    sub_command: "".to_string(),
                    flags: "".to_string(),
                    normalized_command: "ls".to_string(),
                    duration_ms: Some(49),
                    status: "success".to_string(),
                },
            ]
        );
    }

    #[test]
    fn failure_event_and_missing_context_are_recorded() {
        let event = crate::hook::parse(
            r#"{"hook_event_name":"PostToolUseFailure","tool_name":"Bash","tool_input":{"command":"cargo test"}}"#,
        )
        .unwrap();
        let records = build_records(&event, datetime!(2026-07-22 03:00:00 UTC));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, "failure");
        assert_eq!(records[0].project, "");
        assert_eq!(records[0].duration_ms, None);
    }

    #[test]
    fn non_bash_event_builds_no_records() {
        let event = crate::hook::parse(
            r#"{"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"command":"x"}}"#,
        )
        .unwrap();
        assert_eq!(
            build_records(&event, datetime!(2026-07-22 03:00:00 UTC)),
            vec![]
        );
    }
}
