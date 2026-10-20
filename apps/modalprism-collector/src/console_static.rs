//! 控制台静态资源：优先嵌入构建产物，否则返回说明页。

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::Response;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../console-web/dist/"]
struct ConsoleDist;

const FALLBACK_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="utf-8"/><title>ModalPrism 控制台</title>
<style>body{font-family:system-ui,sans-serif;max-width:40rem;margin:3rem auto;padding:0 1rem;line-height:1.5}
code{background:#f4f4f5;padding:.1rem .35rem;border-radius:4px}</style></head>
<body><h1>控制台前端未就绪</h1>
<p>当前二进制未包含 <code>apps/console-web/dist</code>。请在仓库根目录执行：</p>
<pre>cd apps/console-web && npm ci && npm run build
cargo build -p modalprism-collector</pre>
<p>或设置环境变量 <code>MODALPRISM_CONSOLE_DIST</code> 指向已构建的目录。</p>
</body></html>"#;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() || path.ends_with('/') {
        return serve_file("index.html");
    }
    serve_file(path)
}

fn serve_file(path: &str) -> Response {
    if let Some(bytes) = ConsoleDist::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(Body::from(bytes.data.into_owned()))
            .unwrap();
    }
    if path != "index.html" {
        return serve_file("index.html");
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(FALLBACK_HTML))
        .unwrap()
}

pub fn has_embedded_assets() -> bool {
    ConsoleDist::get("index.html").is_some()
}
