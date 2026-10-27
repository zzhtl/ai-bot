//! 模型目录缓存一小时，过期后在下次使用时刷新。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai_bot_agent::{AgentBackend, ModelOption};
use serde::{Deserialize, Serialize};

use crate::store::now_ms;

const HOUR_MS: i64 = 60 * 60 * 1000;

#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    checked_at_ms: i64,
    models: Vec<ModelOption>,
}

pub struct ModelCache {
    file: PathBuf,
    snapshot: Mutex<Option<Snapshot>>,
    backend: Option<Arc<dyn AgentBackend>>,
    refreshing: tokio::sync::Mutex<()>,
}

impl ModelCache {
    pub fn new(file: PathBuf, backend: Option<Arc<dyn AgentBackend>>) -> Self {
        let snapshot = std::fs::read(&file)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok());
        Self {
            file,
            snapshot: Mutex::new(snapshot),
            backend,
            refreshing: tokio::sync::Mutex::new(()),
        }
    }

    pub fn choices(&self) -> Vec<ModelOption> {
        self.snapshot
            .lock()
            .expect("模型缓存锁")
            .as_ref()
            .map(|snapshot| snapshot.models.clone())
            .unwrap_or_default()
    }

    pub fn is_expired(&self) -> bool {
        self.wait(now_ms()).is_zero()
    }

    fn wait(&self, now: i64) -> Duration {
        let millis = self
            .snapshot
            .lock()
            .expect("模型缓存锁")
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .checked_at_ms
                    .saturating_add(HOUR_MS)
                    .saturating_sub(now)
                    .max(0)
            })
            .unwrap_or(0);
        Duration::from_millis(millis as u64)
    }

    async fn refresh(&self, backend: &dyn AgentBackend, now: i64) -> anyhow::Result<()> {
        if !self.wait(now).is_zero() {
            return Ok(());
        }
        let _refreshing = self.refreshing.lock().await;
        if !self.wait(now).is_zero() {
            return Ok(());
        }
        let result = backend.models().await;
        let models = match &result {
            Ok(models) => models.iter().take(5).cloned().collect(),
            Err(_) => self.choices(),
        };
        let snapshot = Snapshot {
            checked_at_ms: now,
            models,
        };
        // 失败也记录本次检查时间，保留上次可用目录，避免反复请求。
        *self.snapshot.lock().expect("模型缓存锁") = Some(snapshot.clone());
        let temporary = self.file.with_extension("json.tmp");
        tokio::fs::write(&temporary, serde_json::to_vec(&snapshot)?).await?;
        tokio::fs::rename(temporary, &self.file).await?;
        result?;
        Ok(())
    }

    pub async fn refresh_if_needed(&self) {
        if let Some(backend) = &self.backend
            && let Err(err) = self.refresh(&**backend, now_ms()).await
        {
            tracing::warn!(%err, "更新模型缓存失败");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_bot_agent::{AgentError, BackendInfo, BackendKind, TurnHandle, TurnRequest};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Backend {
        calls: AtomicUsize,
        fail: AtomicBool,
    }
    #[async_trait::async_trait]
    impl AgentBackend for Backend {
        fn kind(&self) -> BackendKind {
            BackendKind::Codex
        }
        async fn models(&self) -> Result<Vec<ModelOption>, AgentError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            if self.fail.load(Ordering::SeqCst) {
                return Err(AgentError::Preflight("暂时失败".into()));
            }
            Ok((0..7)
                .map(|n| ModelOption {
                    id: format!("model-{call}-{n}"),
                    name: format!("模型 {n}"),
                })
                .collect())
        }
        async fn start_turn(&self, _: TurnRequest) -> Result<TurnHandle, AgentError> {
            panic!("缓存不触发分析")
        }
        async fn preflight(&self) -> Result<BackendInfo, AgentError> {
            panic!("未调用")
        }
    }

    #[tokio::test]
    async fn cache_expires_after_one_hour_and_survives_restarts() {
        let dir = tempfile::tempdir().expect("目录");
        let file = dir.path().join("codex-models.json");
        let backend = Backend {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        };
        let cache = ModelCache::new(file.clone(), None);
        cache.refresh(&backend, 100).await.expect("首次刷新");
        assert_eq!(cache.choices().len(), 5);
        cache
            .refresh(&backend, 100 + HOUR_MS - 1)
            .await
            .expect("一小时内复用");
        let rebooted = ModelCache::new(file, None);
        rebooted.refresh(&backend, 1000).await.expect("重启复用");
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        assert!(rebooted.wait(100 + HOUR_MS).is_zero());
        assert_eq!(
            backend.calls.load(Ordering::SeqCst),
            1,
            "到期本身不触发查询"
        );
        rebooted
            .refresh(&backend, 100 + HOUR_MS)
            .await
            .expect("到期后使用时刷新");
        assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
        assert_eq!(rebooted.choices()[0].id, "model-1-0");
    }

    #[tokio::test]
    async fn failed_refresh_retains_the_previous_choices_without_repeated_queries() {
        let dir = tempfile::tempdir().expect("目录");
        let file = dir.path().join("codex-models.json");
        let backend = Backend {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        };
        let cache = ModelCache::new(file.clone(), None);
        cache.refresh(&backend, 100).await.expect("首次刷新");
        let previous = cache.choices();
        backend.fail.store(true, Ordering::SeqCst);
        assert!(cache.refresh(&backend, 100 + HOUR_MS).await.is_err());
        assert_eq!(cache.choices(), previous);
        let rebooted = ModelCache::new(file, None);
        rebooted
            .refresh(&backend, 1000 + HOUR_MS)
            .await
            .expect("失败后一小时内复用");
        assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
        assert_eq!(rebooted.choices(), previous);
        backend.fail.store(false, Ordering::SeqCst);
        rebooted
            .refresh(&backend, 100 + 2 * HOUR_MS)
            .await
            .expect("下次到期时恢复刷新");
        assert_eq!(backend.calls.load(Ordering::SeqCst), 3);
        assert_eq!(rebooted.choices()[0].id, "model-2-0");
    }

    #[tokio::test]
    async fn concurrent_expired_uses_share_one_refresh() {
        let dir = tempfile::tempdir().expect("目录");
        let cache = ModelCache::new(dir.path().join("codex-models.json"), None);
        let backend = Backend {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        };
        let (first, second) =
            tokio::join!(cache.refresh(&backend, 100), cache.refresh(&backend, 100));
        first.expect("首次刷新");
        second.expect("并发复用");
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        assert_eq!(cache.choices()[0].id, "model-0-0");
    }
}
