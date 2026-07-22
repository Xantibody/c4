use std::path::PathBuf;

use async_trait::async_trait;

use crate::record::NormalizedLog;

use super::Storage;

/// ローカルCSVへのAppend-only保存。ファイルが無ければヘッダ行を書く。
#[derive(Debug)]
pub struct CsvStorage {
    path: PathBuf,
}

impl CsvStorage {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn from_env() -> Self {
        let path = std::env::var("CSV_PATH").unwrap_or_else(|_| "c4.csv".to_string());
        Self::new(path)
    }
}

#[async_trait]
impl Storage for CsvStorage {
    async fn save(&self, logs: &[NormalizedLog]) -> anyhow::Result<()> {
        let write_header = !self.path.exists();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut writer = csv::WriterBuilder::new()
            .has_headers(write_header)
            .from_writer(file);
        for log in logs {
            writer.serialize(log)?;
        }
        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(session: &str, normalized: &str) -> NormalizedLog {
        NormalizedLog {
            timestamp: "2026-07-22T03:00:00Z".to_string(),
            session_id: session.to_string(),
            base_command: normalized.split(' ').next().unwrap().to_string(),
            sub_command: normalized.split(' ').nth(1).unwrap_or("").to_string(),
            flags: "".to_string(),
            normalized_command: normalized.to_string(),
        }
    }

    #[tokio::test]
    async fn writes_header_once_and_appends_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.csv");
        let storage = CsvStorage::new(&path);

        storage.save(&[log("s1", "git commit")]).await.unwrap();
        storage.save(&[log("s2", "ls")]).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(
            lines,
            vec![
                "timestamp,session_id,base_command,sub_command,flags,normalized_command",
                "2026-07-22T03:00:00Z,s1,git,commit,,git commit",
                "2026-07-22T03:00:00Z,s2,ls,,,ls",
            ]
        );
    }
}
