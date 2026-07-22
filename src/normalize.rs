/// 集計キーとなる正規化済みコマンド。引数・パス・機密情報は含まない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedCommand {
    pub base_command: String,
    pub sub_command: String,
    pub normalized: String,
}

/// サブコマンドを持つ主要CLI。第2トークンをsub_commandとして扱う。
const SUBCOMMAND_CLIS: &[&str] = &[
    "git", "npm", "pnpm", "yarn", "docker", "cargo", "aws", "kubectl", "gh", "go", "nix", "just",
];

/// パイプ・論理演算子・逐次実行の区切り。クォート内の同文字列は
/// shell_wordsが1トークンに畳むため誤って区切られない。
const SEPARATORS: &[&str] = &["|", "&&", "||", ";"];

/// コマンド文字列を正規化する。空・パース不能な入力は空のVecを返す。
pub fn normalize(command: &str) -> Vec<NormalizedCommand> {
    let Ok(tokens) = shell_words::split(command) else {
        return vec![];
    };
    tokens
        .split(|t| SEPARATORS.contains(&t.as_str()))
        .filter_map(normalize_segment)
        .collect()
}

fn normalize_segment(tokens: &[String]) -> Option<NormalizedCommand> {
    // `FOO=bar cmd` 形式の先行環境変数代入は機密を含みうるため読み飛ばす
    let tokens = &tokens[tokens
        .iter()
        .position(|t| !is_env_assignment(t))
        .unwrap_or(tokens.len())..];
    let base = tokens.first()?;
    let sub = if SUBCOMMAND_CLIS.contains(&base.as_str()) {
        tokens.get(1).filter(|t| !t.starts_with('-')).cloned()
    } else {
        None
    };
    let normalized = match &sub {
        Some(sub) => format!("{base} {sub}"),
        None => base.clone(),
    };
    Some(NormalizedCommand {
        base_command: base.clone(),
        sub_command: sub.unwrap_or_default(),
        normalized,
    })
}

fn is_env_assignment(token: &str) -> bool {
    token
        .split_once('=')
        .is_some_and(|(name, _)| !name.is_empty() && !name.contains(|c: char| c.is_whitespace()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_command_yields_no_records() {
        assert_eq!(normalize(""), vec![]);
    }

    #[test]
    fn subcommand_cli_takes_second_token_and_drops_secrets() {
        assert_eq!(
            normalize("git commit -m 'feat: secret string'"),
            vec![NormalizedCommand {
                base_command: "git".to_string(),
                sub_command: "commit".to_string(),
                normalized: "git commit".to_string(),
            }]
        );
    }

    #[test]
    fn compound_command_yields_one_record_per_segment() {
        let records = normalize("cat foo.txt | grep bar && git status");
        let normalized: Vec<&str> = records.iter().map(|r| r.normalized.as_str()).collect();
        assert_eq!(normalized, vec!["cat", "grep", "git status"]);
    }

    #[test]
    fn env_var_prefix_is_skipped_and_dropped() {
        assert_eq!(
            normalize("RUST_LOG=debug API_KEY=secret cargo test --lib"),
            vec![NormalizedCommand {
                base_command: "cargo".to_string(),
                sub_command: "test".to_string(),
                normalized: "cargo test".to_string(),
            }]
        );
    }

    #[test]
    fn flag_as_second_token_is_not_a_subcommand() {
        assert_eq!(
            normalize("git --version"),
            vec![NormalizedCommand {
                base_command: "git".to_string(),
                sub_command: "".to_string(),
                normalized: "git".to_string(),
            }]
        );
    }

    #[test]
    fn npm_run_drops_script_options() {
        assert_eq!(
            normalize("npm run dev --port 3000")[0].normalized,
            "npm run"
        );
    }

    #[test]
    fn quoted_separator_is_not_split() {
        let records = normalize("grep 'a|b' file.txt");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].normalized, "grep");
    }

    #[test]
    fn unparseable_quote_yields_no_records() {
        assert_eq!(normalize("echo 'unclosed"), vec![]);
    }

    #[test]
    fn separator_only_input_yields_no_records() {
        assert_eq!(normalize("&&"), vec![]);
    }

    #[test]
    fn single_command_without_subcommand() {
        assert_eq!(
            normalize("ls -la /path/to/dir"),
            vec![NormalizedCommand {
                base_command: "ls".to_string(),
                sub_command: "".to_string(),
                normalized: "ls".to_string(),
            }]
        );
    }
}
