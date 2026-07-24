use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::normalize::normalize;

/// トランスクリプト由来の1ツール呼び出し=1レコード。
/// Bash以外も含む全ツールの「行動」を、タスク文脈を持たない
/// 盲検データとして永続化する。生の引数・パス・プロンプトは残さない。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConductLog {
    /// tool_use発行時刻 (ISO8601 UTC)
    pub timestamp: String,
    pub session_id: String,
    pub tool_use_id: String,
    /// cwdのbasename。フルパスは残さない
    pub project: String,
    /// main / sidechain (サブエージェントの行動)
    pub source: String,
    pub tool_name: String,
    /// ツール別の正規化サマリ。Bashは正規化コマンド連鎖、
    /// Readはoffset/limitの有無、Agentはsubagent_type等。値は含まない
    pub detail: String,
    /// セッションソルト付きFNV-1aハッシュ。パスの中身を明かさずに
    /// 「同一セッション内で同じファイルに触れたか」の等値比較を可能にする
    pub path_hash: String,
    /// 対象ファイルの拡張子 (パスを持つツールのみ)
    pub path_kind: String,
    /// tool_use発行行と結果行のtimestamp差。permission待ちを含む近似値
    pub duration_ms: Option<u64>,
    /// success / failure / interrupted
    pub status: String,
    /// reasoning effortレベル（無ければ空文字）
    pub effort: String,
}

/// ツール入力から (detail, path_hash, path_kind) を導く。
/// 未知のツールは名前だけ記録し、入力は一切残さない（安全側に倒す）。
pub fn normalize_tool_input(tool_name: &str, input: &Value, session_id: &str) -> ToolSummary {
    match tool_name {
        "Bash" => ToolSummary::detail_only(bash_chain(input)),
        "Read" => ToolSummary {
            detail: present_keys(input, &["offset", "limit"]),
            ..ToolSummary::for_path(input, "file_path", session_id)
        },
        "Write" => ToolSummary::for_path(input, "file_path", session_id),
        "Edit" => ToolSummary {
            detail: present_keys(input, &["replace_all"]),
            ..ToolSummary::for_path(input, "file_path", session_id)
        },
        "NotebookEdit" => ToolSummary::for_path(input, "notebook_path", session_id),
        "Grep" => ToolSummary {
            detail: grep_detail(input),
            ..ToolSummary::for_path(input, "path", session_id)
        },
        "Glob" => ToolSummary::for_path(input, "path", session_id),
        "Agent" | "Task" => ToolSummary::detail_only(agent_detail(input)),
        _ => ToolSummary::default(),
    }
}

/// 正規化済みツール入力サマリ。生の値は含まない。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ToolSummary {
    pub detail: String,
    pub path_hash: String,
    pub path_kind: String,
}

impl ToolSummary {
    fn detail_only(detail: String) -> Self {
        Self {
            detail,
            ..Self::default()
        }
    }

    /// 入力中のパスをハッシュ化して保持する。パスが無ければ空のまま
    fn for_path(input: &Value, key: &str, session_id: &str) -> Self {
        let Some(path) = input.get(key).and_then(Value::as_str) else {
            return Self::default();
        };
        Self {
            detail: String::new(),
            path_hash: salted_path_hash(session_id, path),
            path_kind: std::path::Path::new(path)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
        }
    }
}

/// 複合コマンドを正規化して連鎖ごと1文字列に畳む。
/// 例: "git commit -m x && cat f | grep p" → "git commit && cat | grep"
fn bash_chain(input: &Value) -> String {
    let Some(command) = input.get("command").and_then(Value::as_str) else {
        return String::new();
    };
    normalize(command)
        .into_iter()
        .map(|c| {
            if c.connector.is_empty() {
                c.normalized
            } else {
                format!("{} {}", c.connector, c.normalized)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 指定キーのうち入力に存在する（かつfalse/nullでない）ものの名前を列挙する。
/// 値そのものは記録しない
fn present_keys(input: &Value, keys: &[&str]) -> String {
    keys.iter()
        .filter(|k| {
            input
                .get(**k)
                .is_some_and(|v| !v.is_null() && v.as_bool() != Some(false))
        })
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Grepはoutput_modeと使用したオプション名のみ。パターン本文は
/// 検索対象の文字列（機密含みうる）なので一切残さない
fn grep_detail(input: &Value) -> String {
    let mode = input
        .get("output_mode")
        .and_then(Value::as_str)
        .unwrap_or("files_with_matches");
    let opts = present_keys(
        input,
        &[
            "-A",
            "-B",
            "-C",
            "-i",
            "-n",
            "glob",
            "head_limit",
            "multiline",
            "type",
        ],
    );
    if opts.is_empty() {
        mode.to_string()
    } else {
        format!("{mode} {opts}")
    }
}

/// 委譲判断の分析用にsubagent_typeとバックグラウンド実行の有無を残す。
/// promptは残さない
fn agent_detail(input: &Value) -> String {
    let ty = input
        .get("subagent_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let bg = present_keys(input, &["run_in_background"]);
    [ty, &bg]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

/// セッションIDをソルトにしたFNV-1a 64bit。実行間で決定的なので
/// 追記型の収集でも同一ファイルは同一ハッシュに落ちる。
/// セッションを跨ぐと同じパスでも別ハッシュ（辞書攻撃の面を狭める）
fn salted_path_hash(session_id: &str, path: &str) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for byte in session_id.bytes().chain([0u8]).chain(path.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bash_input_normalizes_the_whole_chain() {
        let s = normalize_tool_input(
            "Bash",
            &json!({"command": "git commit -m secret && cat f.txt | grep password"}),
            "sess",
        );
        assert_eq!(s.detail, "git commit && cat | grep");
        assert_eq!(s.path_hash, "");
        assert_eq!(s.path_kind, "");
    }

    #[test]
    fn read_keeps_extension_and_flag_names_but_hashes_path() {
        let s = normalize_tool_input(
            "Read",
            &json!({"file_path": "/Users/me/secret/api_keys.rs", "offset": 10, "limit": 50}),
            "sess-a",
        );
        assert_eq!(s.detail, "offset limit");
        assert_eq!(s.path_kind, "rs");
        assert_eq!(s.path_hash.len(), 16);
        assert!(!s.path_hash.contains("api_keys"));
    }

    #[test]
    fn same_path_same_session_hashes_equal_but_sessions_differ() {
        let input = json!({"file_path": "/repo/src/main.rs"});
        let a1 = normalize_tool_input("Read", &input, "sess-a");
        let a2 = normalize_tool_input("Edit", &input, "sess-a");
        let b = normalize_tool_input("Read", &input, "sess-b");
        assert_eq!(a1.path_hash, a2.path_hash);
        assert_ne!(a1.path_hash, b.path_hash);
    }

    #[test]
    fn edit_records_replace_all_only_when_true() {
        let with = normalize_tool_input(
            "Edit",
            &json!({"file_path": "/a/b.md", "old_string": "x", "replace_all": true}),
            "s",
        );
        let without = normalize_tool_input(
            "Edit",
            &json!({"file_path": "/a/b.md", "old_string": "x", "replace_all": false}),
            "s",
        );
        assert_eq!(with.detail, "replace_all");
        assert_eq!(without.detail, "");
        assert_eq!(with.path_kind, "md");
    }

    #[test]
    fn grep_keeps_mode_and_option_names_never_the_pattern() {
        let s = normalize_tool_input(
            "Grep",
            &json!({"pattern": "AKIA[0-9A-Z]+", "output_mode": "content", "-n": true, "glob": "*.rs"}),
            "s",
        );
        assert_eq!(s.detail, "content -n glob");
        let default_mode = normalize_tool_input("Grep", &json!({"pattern": "x"}), "s");
        assert_eq!(default_mode.detail, "files_with_matches");
    }

    #[test]
    fn agent_records_subagent_type_and_background() {
        let s = normalize_tool_input(
            "Agent",
            &json!({"subagent_type": "Explore", "prompt": "read all the secrets", "run_in_background": true}),
            "s",
        );
        assert_eq!(s.detail, "Explore run_in_background");
        assert_eq!(s.path_hash, "");
    }

    #[test]
    fn unknown_tool_keeps_nothing_from_input() {
        let s = normalize_tool_input(
            "WebFetch",
            &json!({"url": "https://internal.example.com/token", "prompt": "p"}),
            "s",
        );
        assert_eq!(s, ToolSummary::default());
    }

    #[test]
    fn path_hash_is_deterministic_across_calls() {
        assert_eq!(
            salted_path_hash("sess", "/repo/a.rs"),
            salted_path_hash("sess", "/repo/a.rs")
        );
        assert_ne!(
            salted_path_hash("sess", "/repo/a.rs"),
            salted_path_hash("sess", "/repo/b.rs")
        );
    }
}
