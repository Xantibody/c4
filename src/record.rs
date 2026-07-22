use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::hook::HookEvent;
use crate::normalize::normalize;

/// 永続化する1行1レコードのフラット構造。
/// CSV / DuckDB / SQLite での集計を想定し、ネストさせない。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedLog {
    /// 実行日時 (ISO8601 UTC)
    pub timestamp: String,
    pub session_id: String,
    pub base_command: String,
    pub sub_command: String,
    pub normalized_command: String,
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
    normalize(command)
        .into_iter()
        .map(|c| NormalizedLog {
            timestamp: timestamp.clone(),
            session_id: event.session_id.clone(),
            base_command: c.base_command,
            sub_command: c.sub_command,
            normalized_command: c.normalized,
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
                "tool_input": {{"command": {}}}
            }}"#,
            serde_json::to_string(command).unwrap()
        ))
        .unwrap()
    }

    #[test]
    fn builds_flat_records_with_utc_timestamp() {
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
                    base_command: "git".to_string(),
                    sub_command: "commit".to_string(),
                    normalized_command: "git commit".to_string(),
                },
                NormalizedLog {
                    timestamp: "2026-07-22T03:00:00Z".to_string(),
                    session_id: "sess-test".to_string(),
                    base_command: "ls".to_string(),
                    sub_command: "".to_string(),
                    normalized_command: "ls".to_string(),
                },
            ]
        );
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
