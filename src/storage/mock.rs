use std::sync::Mutex;

use async_trait::async_trait;

use crate::record::NormalizedLog;

use super::Storage;

/// テスト用。保存されたレコードをメモリに積むだけ。
#[derive(Debug, Default)]
pub struct MockStorage {
    saved: Mutex<Vec<NormalizedLog>>,
}

impl MockStorage {
    pub fn saved(&self) -> Vec<NormalizedLog> {
        self.saved.lock().expect("mock lock poisoned").clone()
    }
}

#[async_trait]
impl Storage for MockStorage {
    async fn save(&self, logs: &[NormalizedLog]) -> anyhow::Result<()> {
        self.saved
            .lock()
            .expect("mock lock poisoned")
            .extend_from_slice(logs);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(normalized: &str) -> NormalizedLog {
        NormalizedLog {
            timestamp: "2026-07-22T03:00:00Z".to_string(),
            session_id: "sess-test".to_string(),
            base_command: normalized.split(' ').next().unwrap().to_string(),
            sub_command: "".to_string(),
            normalized_command: normalized.to_string(),
        }
    }

    #[tokio::test]
    async fn mock_accumulates_saved_logs() {
        let storage = MockStorage::default();
        storage.save(&[log("ls"), log("git")]).await.unwrap();
        assert_eq!(storage.saved().len(), 2);
    }
}
