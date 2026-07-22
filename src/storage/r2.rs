use async_trait::async_trait;
use aws_sdk_s3::primitives::ByteStream;

use crate::record::NormalizedLog;

use super::Storage;

/// Cloudflare R2 (S3互換) への保存。
/// S3系ストレージは追記ができないため、1回のhook呼び出しごとに
/// JSONLの小オブジェクトを1つPUTすることでAppend-onlyを実現する。
/// キーは日付パーティション形式で、DuckDB等からまとめて読める。
pub struct R2Storage {
    client: aws_sdk_s3::Client,
    bucket: String,
}

impl R2Storage {
    pub async fn from_env() -> anyhow::Result<Self> {
        let bucket = std::env::var("R2_BUCKET")
            .map_err(|_| anyhow::anyhow!("R2_BUCKET must be set for STORAGE_TYPE=r2"))?;
        let endpoint = std::env::var("R2_ENDPOINT")
            .map_err(|_| anyhow::anyhow!("R2_ENDPOINT must be set for STORAGE_TYPE=r2"))?;
        let config = aws_config::from_env()
            .endpoint_url(endpoint)
            .region("auto")
            .load()
            .await;
        Ok(Self {
            client: aws_sdk_s3::Client::new(&config),
            bucket,
        })
    }

    /// 衝突しないオブジェクトキーを組み立てる。
    /// 例: logs/dt=2026-07-22/1753150000000000000-sess-xxxx.jsonl
    fn object_key(log: &NormalizedLog) -> String {
        let date = log.timestamp.split('T').next().unwrap_or("unknown");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("logs/dt={date}/{nanos}-{}.jsonl", log.session_id)
    }
}

#[async_trait]
impl Storage for R2Storage {
    async fn save(&self, logs: &[NormalizedLog]) -> anyhow::Result<()> {
        let Some(first) = logs.first() else {
            return Ok(());
        };
        let mut body = String::new();
        for log in logs {
            body.push_str(&serde_json::to_string(log)?);
            body.push('\n');
        }
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(Self::object_key(first))
            .content_type("application/x-ndjson")
            .body(ByteStream::from(body.into_bytes()))
            .send()
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_key_is_date_partitioned() {
        let log = NormalizedLog {
            timestamp: "2026-07-22T03:00:00Z".to_string(),
            session_id: "sess-xxxx".to_string(),
            project: "c4".to_string(),
            base_command: "git".to_string(),
            sub_command: "commit".to_string(),
            flags: "-m".to_string(),
            normalized_command: "git commit".to_string(),
            duration_ms: Some(49),
            status: "success".to_string(),
        };
        let key = R2Storage::object_key(&log);
        assert!(key.starts_with("logs/dt=2026-07-22/"));
        assert!(key.ends_with("-sess-xxxx.jsonl"));
    }
}
