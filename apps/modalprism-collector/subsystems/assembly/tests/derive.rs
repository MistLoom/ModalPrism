//! 派生执行接线：media-download 产出方的离线验证。
//!
//! 全程不出网：本地 TcpListener 充当文件通道，只响应固定字节。
//! 覆盖两条口径——声明齐备时下载并写成二进制 facet，超出 media.max_size 时确认跳过。

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use assembly::{Binds, Services};
use config::builtin_presets;
use serde_json::json;
use strata_model::{
    ApplyOpts, Body, ChangeSet, FacetKind, FacetRef, LineageInput, Op, Producer, Rid, Target,
    VersionSel,
};

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// 媒体策略写在自定义预设里：预设的生效值高于 system.yaml 同名键。
fn config_dir(dir: &Path, max_size: &str) {
    for preset in builtin_presets() {
        write(dir, &format!("presets/{}", preset.file_name), preset.source);
    }
    let tiny = PRESET_TEMPLATE.replace("MAX_SIZE_HERE", max_size);
    write(dir, "presets/tiny.yaml", &tiny);
    write(dir, "system.yaml", SYSTEM_BODY);
}

const SYSTEM_BODY: &str = r"id: system
kind: system
name: ModalPrism
policy:
  preset: tiny
  egress:
    private_networks: allow
strata:
  gateway:
    blob_backend: dir
    derivations:
      - name: media-download
        from:
          kind: struct
          variant: imglink
          fields: [struct]
        produces: image
        producer:
          module: media-download
          version: '1'
          configGeneration: 1
        queue: render
";

const PRESET_TEMPLATE: &str = r"id: preset/tiny
kind: preset
name: Tiny
extends: [preset/standard]
policy:
  media:
    mode: download
    kinds: [image]
    max_size: MAX_SIZE_HERE
";

fn binds() -> Binds {
    Binds {
        http: "127.0.0.1:0".parse().unwrap(),
        proxy: "127.0.0.1:0".parse().unwrap(),
        push: "127.0.0.1:0".parse().unwrap(),
    }
}

fn producer() -> Producer {
    Producer {
        module: "file-channel-test".into(),
        version: "1".into(),
        config_generation: 1,
    }
}

/// 本地文件通道：robots 一律放行，其余返回固定的 64 字节图片。
async fn file_channel() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let text = String::from_utf8_lossy(&buf).into_owned();
                let body: Vec<u8> = if text.contains("robots.txt") {
                    b"User-agent: *\nAllow: /\n".to_vec()
                } else {
                    (0..64u8).collect()
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let mut response = head.into_bytes();
                response.extend_from_slice(&body);
                let _ = socket.write_all(&response).await;
            });
        }
    });
    (addr, task)
}

/// 写入带地址的结构化条目并发布：提交侧自动把派生任务排进队列。
async fn seed_input(services: &Arc<Services>, addr: SocketAddr) -> Rid {
    let value = json!({ "url": format!("http://{addr}/i.png"), "title": "配图条目" });
    services
        .strata()
        .apply(
            ChangeSet {
                producer: producer(),
                idempotency_key: None,
                ack: None,
                ops: vec![
                    Op::Ensure {
                        identity: "ext:derive:media-1".into(),
                        ns: None,
                        labels: Default::default(),
                        r#as: "a".into(),
                    },
                    Op::PutFacet {
                        target: Target::Alias("a".into()),
                        kind: FacetKind::Struct,
                        variant: Some("imglink".into()),
                        body: Body::Struct { value },
                        media_type: Some("application/json".into()),
                        lineage: LineageInput {
                            inputs: Vec::new(),
                            params_hash: "seed".into(),
                        },
                    },
                ],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    services.committer_flush().await.unwrap();
    services
        .strata()
        .resolve("ext:derive:media-1")
        .await
        .unwrap()
        .unwrap()
}

fn derived(rid: &Rid) -> FacetRef {
    FacetRef::new(rid.clone(), FacetKind::Image, None, VersionSel::Latest).unwrap()
}

#[tokio::test]
async fn media_download_writes_a_binary_facet_from_the_local_channel() {
    let (addr, _channel) = file_channel().await;
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path(), "1KiB");
    let services = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    let rid = seed_input(&services, addr).await;

    let run = services.run_derivations().await.unwrap();
    assert_eq!(run.leased, 1, "{run:?}");
    assert_eq!(run.produced, 1, "{run:?}");
    assert_eq!((run.skipped, run.failed), (0, 0));

    let facet = services
        .strata()
        .get(&derived(&rid), false)
        .await
        .unwrap()
        .expect("应写成 image facet");
    let bytes = facet.bytes.expect("二进制 facet 应带回读字节");
    assert_eq!(bytes.as_ref(), (0..64u8).collect::<Vec<u8>>().as_slice());
}

#[tokio::test]
async fn media_download_skips_when_the_target_exceeds_max_size() {
    let (addr, _channel) = file_channel().await;
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    // 文件通道固定返回 64 字节，上限收到 32 字节即应确认跳过而不是写库。
    config_dir(config.path(), "32B");
    let services = Services::open(data.path(), config.path(), binds())
        .await
        .unwrap();
    let rid = seed_input(&services, addr).await;

    let run = services.run_derivations().await.unwrap();
    assert_eq!(run.leased, 1, "{run:?}");
    assert_eq!(run.skipped, 1, "{run:?}");
    assert_eq!((run.produced, run.failed), (0, 0));
    assert!(
        services
            .strata()
            .get(&derived(&rid), false)
            .await
            .unwrap()
            .is_none(),
        "超限不应写出 image facet"
    );
}
