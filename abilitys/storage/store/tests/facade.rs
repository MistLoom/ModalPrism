use ability_core::AbilityHost;
use ability_support::{InstanceId, LoadSpec, SourceId};
use frame_port_fun::FunPort;
use serde_json::json;

async fn boot(root: &std::path::Path) -> FunPort {
    let host = AbilityHost::new();
    host.install_str("store", ability_store::provider())
        .await
        .unwrap();
    host.install_str("store.persist", ability_store_persist::provider())
        .await
        .unwrap();
    host.install_str("store.temp", ability_store_temp::provider())
        .await
        .unwrap();
    host.install_str("markdown", ability_markdown::provider())
        .await
        .unwrap();
    host.load(
        LoadSpec::new(
            SourceId::new("store").unwrap(),
            InstanceId::new("store").unwrap(),
        )
        .with_config(json!({
            "data_root": root.join("data").to_string_lossy(),
            "temp_root": root.join("tmp").to_string_lossy(),
            "watch": false
        })),
    )
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("store.persist").unwrap(),
        InstanceId::new("store.persist").unwrap(),
    ))
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("store.temp").unwrap(),
        InstanceId::new("store.temp").unwrap(),
    ))
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("markdown").unwrap(),
        InstanceId::new("markdown").unwrap(),
    ))
    .await
    .unwrap();
    FunPort::new(host.into_gateway())
}

#[tokio::test]
async fn yaml_json_sqlite_and_stream() {
    let root = std::env::temp_dir().join(format!(
        "leaf-store-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let fun = boot(&root).await;

    fun.invoke_str(
        "store.put",
        json!({
            "owner": "demo",
            "key": "cfg.yaml",
            "value": { "port": 8, "name": "n" },
            "codec": "yaml"
        }),
        None,
    )
    .await
    .unwrap();
    let got = fun
        .invoke_str(
            "store.get",
            json!({ "owner": "demo", "key": "cfg.yaml", "codec": "yaml" }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(got.data["value"]["port"], 8);

    fun.invoke_str(
        "store.put",
        json!({
            "owner": "demo",
            "key": "row",
            "value": { "n": 1 },
            "codec": "sqlite"
        }),
        None,
    )
    .await
    .unwrap();
    let row = fun
        .invoke_str(
            "store.get",
            json!({ "owner": "demo", "key": "row", "codec": "sqlite" }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(row.data["value"]["n"], 1);

    use base64::Engine;
    let raw = vec![1u8, 2, 3, 4];
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    fun.invoke_str(
        "store.blob_write",
        json!({
            "owner": "demo",
            "key": "big.bin",
            "offset": 0,
            "data": b64,
            "last": true
        }),
        None,
    )
    .await
    .unwrap();
    let blob = fun
        .invoke_str(
            "store.blob_get",
            json!({ "owner": "demo", "key": "big.bin", "offset": 0 }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(blob.data["eof"], true);

    fun.invoke_str(
        "store.persist.put",
        json!({ "owner": "demo", "key": "p.json", "value": { "ok": true } }),
        None,
    )
    .await
    .unwrap();
    fun.invoke_str(
        "store.temp.put",
        json!({ "owner": "demo", "key": "p.json", "value": { "ok": false } }),
        None,
    )
    .await
    .unwrap();
    let a = fun
        .invoke_str(
            "store.persist.get",
            json!({ "owner": "demo", "key": "p.json" }),
            None,
        )
        .await
        .unwrap();
    let b = fun
        .invoke_str(
            "store.temp.get",
            json!({ "owner": "demo", "key": "p.json" }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(a.data["value"]["ok"], true);
    assert_eq!(b.data["value"]["ok"], false);

    fun.invoke_str(
        "markdown.write",
        json!({
            "owner": "demo",
            "key": "note.md",
            "frontmatter": { "title": "t" },
            "body": "# H\nhello"
        }),
        None,
    )
    .await
    .unwrap();
    let read = fun
        .invoke_str(
            "markdown.read",
            json!({ "owner": "demo", "key": "note.md" }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(read.data["frontmatter"]["title"], "t");
    assert_eq!(read.data["headings"][0]["text"], "H");

    let _ = std::fs::remove_dir_all(root);
}
