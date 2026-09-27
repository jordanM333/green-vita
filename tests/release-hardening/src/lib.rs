#![cfg(test)]
#[path = "../../../src/api/catalog/cache.rs"]
mod cache;
#[path = "../../../src/fs_utils.rs"]
pub mod fs_utils;
#[path = "../../../src/http.rs"]
pub mod http;
#[path = "../../../src/resource_limits.rs"]
pub mod resource_limits;
#[cfg(test)]
mod tests;

#[path = "../../../src/jobs.rs"]
mod jobs;

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct GameDetails;
#[derive(Clone)]
pub struct GameCatalogBackend {
    pub entered: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub pending: bool,
}
impl GameCatalogBackend {
    pub fn cache_namespace(&self) -> &'static str {
        "shutdown-fixture"
    }
    pub async fn fetch_details(
        &self,
        _: &reqwest::Client,
        _: &str,
        _: &str,
        _: &str,
    ) -> anyhow::Result<GameDetails> {
        self.entered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if self.pending {
            std::future::pending::<()>().await;
        }
        anyhow::bail!("fixture metadata unavailable")
    }
}
#[path = "../../../src/api/catalog/worker.rs"]
pub mod worker;

extern crate self as vitasdk_sys;
mod sdk;
pub use sdk::{
    SceAppUtilBootParam, SceAppUtilInitParam, sceAppUtilInit, sceAppUtilLoadSafeMemory,
    sceAppUtilSaveSafeMemory, sceAppUtilShutdown,
};
#[path = "../../../src/api_xbox/auth.rs"]
pub mod auth;
#[path = "../../../src/safe_memory.rs"]
pub mod safe_memory;

#[path = "../../../src/api/streaming/rtc/failure_budget.rs"]
mod failure_budget;
