/// 集計キーとなる正規化済みコマンド。引数・パス・機密情報は含まない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedCommand {
    pub base_command: String,
    pub sub_command: String,
    pub normalized: String,
}

/// コマンド文字列を正規化する。空・パース不能な入力は空のVecを返す。
/// サブコマンドを持つ主要CLI。第2トークンをsub_commandとして扱う。
const SUBCOMMAND_CLIS: &[&str] = &[
    "git", "npm", "pnpm", "yarn", "docker", "cargo", "aws", "kubectl", "gh", "go", "nix", "just",
];

pub fn normalize(command: &str) -> Vec<NormalizedCommand> {
    let Ok(tokens) = shell_words::split(command) else {
        return vec![];
    };
    let Some(base) = tokens.first() else {
        return vec![];
    };
    let sub = if SUBCOMMAND_CLIS.contains(&base.as_str()) {
        tokens.get(1).filter(|t| !t.starts_with('-')).cloned()
    } else {
        None
    };
    let normalized = match &sub {
        Some(sub) => format!("{base} {sub}"),
        None => base.clone(),
    };
    vec![NormalizedCommand {
        base_command: base.clone(),
        sub_command: sub.unwrap_or_default(),
        normalized,
    }]
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
