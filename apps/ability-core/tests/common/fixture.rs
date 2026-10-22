//! Guest 合规测试装配。

use std::path::PathBuf;
use std::sync::Arc;

use ability_core::{AbilityHost, GuestSourceProvider, connect};
use ability_support::{
    Ability, AbilityManifest, AbilityProvider, InstanceId, LoadSpec, Result, SourceId,
};
use async_trait::async_trait;
use serde_json::Value;
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn sid(s: &str) -> SourceId {
    SourceId::new(s).unwrap()
}

pub fn iid(s: &str) -> InstanceId {
    InstanceId::new(s).unwrap()
}

pub fn mid(s: &str) -> ability_support::MethodId {
    ability_support::MethodId::new(s).unwrap()
}

pub async fn install_guest_subprocess(host: &AbilityHost, source: &str, rel: &str) {
    let provider = GuestSourceProvider::from_workspace_rel(&workspace_root(), rel).into_provider();
    host.install(sid(source), provider).await;
    host.load(LoadSpec::new(sid(source), iid(source)))
        .await
        .unwrap();
}

pub async fn install_guest_provider_only(host: &AbilityHost, source: &str, rel: &str) {
    let provider = GuestSourceProvider::from_workspace_rel(&workspace_root(), rel).into_provider();
    host.install(sid(source), provider).await;
}

pub async fn boot_duplex(provider: Arc<dyn AbilityProvider>, config: Value) -> Arc<dyn Ability> {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (server_r, server_w) = tokio::io::split(server);
    let (client_r, client_w) = tokio::io::split(client);
    tokio::spawn(async move {
        ability_sdk::serve(provider, server_r, server_w)
            .await
            .unwrap();
    });
    connect(client_r, client_w, config).await.unwrap()
}

/// 每次 create 开一条内存 duplex，客端走 `ability-sdk::serve`。
struct DuplexProvider {
    inner: Arc<dyn AbilityProvider>,
}

#[async_trait]
impl AbilityProvider for DuplexProvider {
    fn manifest(&self) -> AbilityManifest {
        self.inner.manifest()
    }

    async fn create(&self, config: Value) -> Result<Arc<dyn Ability>> {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (server_r, server_w) = tokio::io::split(server);
        let (client_r, client_w) = tokio::io::split(client);
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let _ = ability_sdk::serve(inner, server_r, server_w).await;
        });
        connect(client_r, client_w, config).await
    }
}

pub fn duplex_provider(inner: Arc<dyn AbilityProvider>) -> Arc<dyn AbilityProvider> {
    Arc::new(DuplexProvider { inner })
}

pub async fn install_connected(host: &AbilityHost, source: &str, ability: Arc<dyn Ability>) {
    host.install(
        sid(source),
        Arc::new(ability_support::StaticProvider::new(ability)),
    )
    .await;
    host.load(LoadSpec::new(sid(source), iid(source)))
        .await
        .unwrap();
}
