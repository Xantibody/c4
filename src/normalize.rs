/// 集計キーとなる正規化済みコマンド。引数・パス・機密情報は含まない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedCommand {
    pub base_command: String,
    pub sub_command: String,
    pub normalized: String,
}

/// コマンド文字列を正規化する。空・パース不能な入力は空のVecを返す。
pub fn normalize(command: &str) -> Vec<NormalizedCommand> {
    let Ok(tokens) = shell_words::split(command) else {
        return vec![];
    };
    let Some(base) = tokens.first() else {
        return vec![];
    };
    vec![NormalizedCommand {
        base_command: base.clone(),
        sub_command: String::new(),
        normalized: base.clone(),
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
