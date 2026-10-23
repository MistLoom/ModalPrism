//! 前端产物托管。单页应用：找不到的路径交回 `index.html`。

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use crate::Console;

pub async fn serve(State(console): State<Arc<Console>>, uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    let Some(path) = resolve(&console.web, requested) else {
        return not_found();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, content_type(&path)),
                (header::CACHE_CONTROL, cache_control(&path)),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => {
            if requested == "index.html" {
                return missing_build();
            }
            // 目录型路由交回入口，由前端自己决定渲染什么。
            let index = console.web.join("index.html");
            match tokio::fs::read(index).await {
                Ok(bytes) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    bytes,
                )
                    .into_response(),
                Err(_) => missing_build(),
            }
        }
    }
}

/// 只接受相对路径，任何越级片段都按不存在处理。
fn resolve(root: &Path, requested: &str) -> Option<PathBuf> {
    let relative = Path::new(requested);
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    Some(root.join(relative))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// 带哈希的产物可以长缓存；入口不缓存，保证升级即生效。
fn cache_control(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if name == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "资源不存在").into_response()
}

fn missing_build() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "前端产物尚未构建：在 apps/modalprism-console/web 执行 pnpm install && pnpm build",
    )
        .into_response()
}
