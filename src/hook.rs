use serde::Deserialize;

/// Claude Code の PostToolUse / PostToolUseFailure hook が stdin に渡すJSON。
/// 実際のClaude Codeは `hook_event_name`/`tool_name`/`tool_input` を使うが、
/// 簡略形式 `hook`/`tool`/`input` もaliasで受ける。
/// cwd / duration_ms は実ペイロードで確認済み（公式ドキュメントには
/// duration_msの明記がないため、欠けていても動くようOptionで受ける）。
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct HookEvent {
    #[serde(alias = "hook")]
    pub hook_event_name: String,
    #[serde(alias = "tool")]
    pub tool_name: String,
    #[serde(default)]
    pub session_id: String,
    /// 1回のBashツール呼び出しを識別するID。複合コマンドから生じた
    /// 複数レコードのグループ化キーになる
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// reasoning effort設定。modelはペイロードに含まれないため、
    /// 「どの思考設定での実行か」の代理変数として記録する
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(alias = "input")]
    pub tool_input: ToolInput,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Effort {
    pub level: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct ToolInput {
    pub command: String,
}

impl HookEvent {
    /// Bashツールのコマンドのみ収集対象とする
    pub fn bash_command(&self) -> Option<&str> {
        (self.tool_name == "Bash").then_some(self.tool_input.command.as_str())
    }

    /// effortレベル。ペイロードに無ければ空文字
    pub fn effort_level(&self) -> &str {
        self.effort.as_ref().map_or("", |e| e.level.as_str())
    }

    /// 発火イベント名から成否を導く。成功時はPostToolUse、
    /// 失敗時はPostToolUseFailureに同じhookを登録する前提。
    pub fn status(&self) -> &'static str {
        if self.hook_event_name == "PostToolUseFailure" {
            "failure"
        } else {
            "success"
        }
    }
}

pub fn parse(json: &str) -> anyhow::Result<HookEvent> {
    Ok(serde_json::from_str(json)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_alias_format() {
        let event = parse(
            r#"{
                "hook": "PostToolUse",
                "tool": "Bash",
                "input": {"command": "ls -la"},
                "session_id": "sess-yyyy"
            }"#,
        )
        .unwrap();
        assert_eq!(event.bash_command(), Some("ls -la"));
        assert_eq!(event.cwd, "");
        assert_eq!(event.duration_ms, None);
        assert_eq!(event.effort_level(), "");
    }

    #[test]
    fn parses_cwd_and_duration() {
        let event = parse(
            r#"{
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "session_id": "sess-xxxx",
                "cwd": "/Users/me/Repository/c4",
                "duration_ms": 49,
                "tool_use_id": "toolu_abc",
                "effort": {"level": "high"},
                "tool_input": {"command": "ls"}
            }"#,
        )
        .unwrap();
        assert_eq!(event.cwd, "/Users/me/Repository/c4");
        assert_eq!(event.duration_ms, Some(49));
        assert_eq!(event.tool_use_id, "toolu_abc");
        assert_eq!(event.effort_level(), "high");
        assert_eq!(event.status(), "success");
    }

    #[test]
    fn failure_event_reports_failure_status() {
        let event = parse(
            r#"{
                "hook_event_name": "PostToolUseFailure",
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test"}
            }"#,
        )
        .unwrap();
        assert_eq!(event.status(), "failure");
        assert_eq!(event.bash_command(), Some("cargo test"));
    }

    #[test]
    fn non_bash_tool_is_not_collected() {
        let event = parse(
            r#"{
                "hook_event_name": "PostToolUse",
                "tool_name": "Read",
                "session_id": "sess-zzzz",
                "tool_input": {"command": "dummy"}
            }"#,
        )
        .unwrap();
        assert_eq!(event.bash_command(), None);
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse("not json").is_err());
    }

    #[test]
    fn parses_claude_code_native_format() {
        let event = parse(
            r#"{
                "session_id": "sess-xxxx",
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": "git status", "description": "Show status"},
                "tool_response": {"stdout": "clean"}
            }"#,
        )
        .unwrap();
        assert_eq!(event.session_id, "sess-xxxx");
        assert_eq!(event.bash_command(), Some("git status"));
    }
}
