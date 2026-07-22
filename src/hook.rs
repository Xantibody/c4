use serde::Deserialize;

/// Claude Code の PostToolUse hook が stdin に渡すJSON。
/// 実際のClaude Codeは `hook_event_name`/`tool_name`/`tool_input` を使うが、
/// 簡略形式 `hook`/`tool`/`input` もaliasで受ける。
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct HookEvent {
    #[serde(alias = "hook")]
    pub hook_event_name: String,
    #[serde(alias = "tool")]
    pub tool_name: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(alias = "input")]
    pub tool_input: ToolInput,
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
