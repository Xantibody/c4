mod csv;
mod mock;
mod r2;

pub use csv::{CsvStorage, append as csv_append};
pub use mock::MockStorage;
pub use r2::R2Storage;

use async_trait::async_trait;

use crate::record::NormalizedLog;

/// レコード永続化の抽象。1回のhook呼び出しで生じた
/// 複数レコードをまとめてAppend-onlyで保存する。
#[async_trait]
pub trait Storage {
    async fn save(&self, logs: &[NormalizedLog]) -> anyhow::Result<()>;
}

/// STORAGE_TYPE環境変数 (r2 / csv / mock) から実装を選ぶ。
/// 未指定はローカルで安全なcsvをデフォルトとする。
pub async fn from_env() -> anyhow::Result<Box<dyn Storage>> {
    match std::env::var("STORAGE_TYPE").as_deref() {
        Ok("r2") => Ok(Box::new(R2Storage::from_env().await?)),
        Ok("mock") => Ok(Box::new(MockStorage::default())),
        Ok("csv") | Err(_) => Ok(Box::new(CsvStorage::from_env())),
        Ok(other) => anyhow::bail!("unknown STORAGE_TYPE: {other}"),
    }
}
