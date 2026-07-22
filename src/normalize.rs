/// 集計キーとなる正規化済みコマンド。引数・パス・機密情報は含まない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedCommand {
    pub base_command: String,
    pub sub_command: String,
    /// ソート・重複排除済みのフラグ名。値は含まない (例: ["--amend", "-m"])
    pub flags: Vec<String>,
    pub normalized: String,
}

/// サブコマンドを持つ主要CLI。グローバルオプションを読み飛ばした
/// 最初の非フラグトークンをsub_commandとして扱う。
const SUBCOMMAND_CLIS: &[&str] = &[
    "git", "npm", "pnpm", "yarn", "docker", "cargo", "aws", "kubectl", "gh", "go", "nix", "just",
];

/// コマンド文字列を正規化する。空のセグメントや
/// パース不能なセグメントはレコードを生成しない。
pub fn normalize(command: &str) -> Vec<NormalizedCommand> {
    split_segments(command)
        .into_iter()
        .filter_map(|seg| shell_words::split(&seg).ok())
        .filter_map(|tokens| normalize_segment(&tokens))
        .collect()
}

/// クォート外の `;` `|` `||` `&&` で複合コマンドを分割する。
/// トークン化前の生文字列を走査するため、`echo hi;ls` のような
/// 密着した区切りも扱える。`2>&1` の単独 `&` は区切りとしない。
fn split_segments(command: &str) -> Vec<String> {
    let mut segments = vec![String::new()];
    let mut chars = command.chars().peekable();
    let (mut in_single, mut in_double) = (false, false);
    while let Some(c) = chars.next() {
        let quoted = in_single || in_double;
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '\\' if !in_single => {
                segments.last_mut().expect("never empty").push(c);
                if let Some(next) = chars.next() {
                    segments.last_mut().expect("never empty").push(next);
                }
                continue;
            }
            ';' | '|' if !quoted => {
                segments.push(String::new());
                continue;
            }
            '&' if !quoted && chars.peek() == Some(&'&') => {
                chars.next();
                segments.push(String::new());
                continue;
            }
            _ => {}
        }
        segments.last_mut().expect("never empty").push(c);
    }
    segments
}

fn normalize_segment(tokens: &[String]) -> Option<NormalizedCommand> {
    // `FOO=bar cmd` 形式の先行環境変数代入は機密を含みうるため読み飛ばす
    let tokens = &tokens[tokens
        .iter()
        .position(|t| !is_env_assignment(t))
        .unwrap_or(tokens.len())..];
    let base = tokens.first()?;
    let sub = find_subcommand(base, &tokens[1..]);
    let normalized = match &sub {
        Some(sub) => format!("{base} {sub}"),
        None => base.clone(),
    };
    Some(NormalizedCommand {
        base_command: base.clone(),
        sub_command: sub.unwrap_or_default(),
        flags: collect_flags(&tokens[1..]),
        normalized,
    })
}

/// サブコマンドより前に置かれ、値を別トークンで取るグローバルフラグ。
/// これを知らないと `git -C /path commit` の /path をサブコマンドと
/// 誤認するか、commit を見逃す。
fn value_taking_global_flags(base: &str) -> &'static [&'static str] {
    match base {
        "git" => &[
            "-C",
            "-c",
            "--git-dir",
            "--work-tree",
            "--namespace",
            "--exec-path",
        ],
        "docker" => &["-H", "--host", "--context", "--config", "-l", "--log-level"],
        "kubectl" => &[
            "-n",
            "--namespace",
            "--context",
            "--kubeconfig",
            "--cluster",
            "--user",
            "-s",
            "--server",
        ],
        "npm" | "pnpm" | "yarn" => &["--prefix", "-C", "--dir"],
        "cargo" => &["--color", "--config", "-Z"],
        "just" => &["-f", "--justfile", "-d", "--working-directory"],
        "nix" => &["--option", "--log-format"],
        _ => &[],
    }
}

/// サブコマンドを探す。グローバルフラグ（値取りは値ごと）を
/// 読み飛ばし、最初の非フラグトークンをサブコマンドとする。
fn find_subcommand(base: &str, tokens: &[String]) -> Option<String> {
    if !SUBCOMMAND_CLIS.contains(&base) {
        return None;
    }
    let value_flags = value_taking_global_flags(base);
    let mut iter = tokens.iter();
    while let Some(token) = iter.next() {
        if value_flags.contains(&token.as_str()) {
            iter.next();
            continue;
        }
        if token.starts_with('-') {
            continue;
        }
        return Some(token.clone());
    }
    None
}

/// フラグ名だけを安全側に倒して収集する。
/// - `--name=value` は `=` の手前まで（名前だけ）
/// - 短フラグは英字のみ3文字以内 (`-m` `-la` `-rf`) ならそのまま、
///   くっつき値の可能性がある長いもの (`-psecret`) は先頭1文字に切り詰め、
///   英字で始まらないもの (`-5`) は値とみなして捨てる
/// - 裸の `--` 以降はオペランドなので収集しない
fn collect_flags(tokens: &[String]) -> Vec<String> {
    let mut flags: Vec<String> = tokens
        .iter()
        .take_while(|t| t.as_str() != "--")
        .filter_map(|t| flag_name(t))
        .collect();
    flags.sort();
    flags.dedup();
    flags
}

fn flag_name(token: &str) -> Option<String> {
    if let Some(body) = token.strip_prefix("--") {
        // `---` のような英数字で始まらないトークンはフラグではなくオペランド
        if !body.starts_with(|c: char| c.is_ascii_alphanumeric()) {
            return None;
        }
        let name = body.split('=').next().expect("split yields at least one");
        return Some(format!("--{name}"));
    }
    let body = token.strip_prefix('-')?;
    let mut chars = body.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if body.len() <= 2 && body.chars().all(|c| c.is_ascii_alphabetic()) {
        Some(token.to_string())
    } else {
        Some(format!("-{first}"))
    }
}

fn is_env_assignment(token: &str) -> bool {
    token
        .split_once('=')
        .is_some_and(|(name, _)| !name.is_empty() && !name.contains(|c: char| c.is_whitespace()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(command: &str) -> Vec<String> {
        normalize(command).remove(0).flags
    }

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
                flags: vec!["-m".to_string()],
                normalized: "git commit".to_string(),
            }]
        );
    }

    #[test]
    fn flags_are_sorted_and_deduped_without_values() {
        assert_eq!(
            flags("git commit --amend -m 'x' -m 'y'"),
            vec!["--amend", "-m"]
        );
    }

    #[test]
    fn dash_only_tokens_are_not_flags() {
        assert_eq!(flags("echo --- output"), Vec::<String>::new());
        assert_eq!(flags("echo ----"), Vec::<String>::new());
    }

    #[test]
    fn long_flag_value_after_equals_is_dropped() {
        assert_eq!(flags("mysql --password=secret123"), vec!["--password"]);
    }

    #[test]
    fn short_flag_bundle_is_kept() {
        assert_eq!(flags("ls -la"), vec!["-la"]);
        assert_eq!(flags("rm -rf dir"), vec!["-rf"]);
    }

    #[test]
    fn attached_short_flag_value_is_truncated_to_first_letter() {
        assert_eq!(flags("mysql -psecret123"), vec!["-p"]);
    }

    #[test]
    fn numeric_short_token_is_treated_as_value() {
        assert_eq!(flags("head -5 file.txt"), Vec::<String>::new());
    }

    #[test]
    fn tokens_after_bare_double_dash_are_operands() {
        assert_eq!(flags("git checkout -b topic -- -weird-file"), vec!["-b"]);
    }

    #[test]
    fn attached_separator_splits_segments() {
        let records = normalize("echo ---; ls -la");
        let normalized: Vec<&str> = records.iter().map(|r| r.normalized.as_str()).collect();
        assert_eq!(normalized, vec!["echo", "ls"]);
    }

    #[test]
    fn attached_pipe_splits_segments() {
        let records = normalize("cat foo.txt|grep bar");
        let normalized: Vec<&str> = records.iter().map(|r| r.normalized.as_str()).collect();
        assert_eq!(normalized, vec!["cat", "grep"]);
    }

    #[test]
    fn stream_redirect_ampersand_is_not_a_separator() {
        let records = normalize("cargo build 2>&1");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].normalized, "cargo build");
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
                flags: vec!["--lib".to_string()],
                normalized: "cargo test".to_string(),
            }]
        );
    }

    #[test]
    fn subcommand_is_found_after_value_taking_global_flag() {
        assert_eq!(
            normalize("git -C /path/to/repo commit -m secret"),
            vec![NormalizedCommand {
                base_command: "git".to_string(),
                sub_command: "commit".to_string(),
                flags: vec!["-C".to_string(), "-m".to_string()],
                normalized: "git commit".to_string(),
            }]
        );
    }

    #[test]
    fn subcommand_is_found_after_boolean_global_flag() {
        assert_eq!(
            normalize("git --no-pager log --oneline")[0].normalized,
            "git log"
        );
        assert_eq!(
            normalize("kubectl -n prod get pods")[0].normalized,
            "kubectl get"
        );
    }

    #[test]
    fn global_flag_value_is_not_mistaken_for_subcommand() {
        // -C の値 /path はサブコマンドではない
        assert_eq!(normalize("git -C /path/to/repo")[0].normalized, "git");
    }

    #[test]
    fn flag_as_second_token_is_not_a_subcommand() {
        assert_eq!(
            normalize("git --version"),
            vec![NormalizedCommand {
                base_command: "git".to_string(),
                sub_command: "".to_string(),
                flags: vec!["--version".to_string()],
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
                flags: vec!["-la".to_string()],
                normalized: "ls".to_string(),
            }]
        );
    }
}
