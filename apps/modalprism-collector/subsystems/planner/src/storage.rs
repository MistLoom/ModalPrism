//! M-05 计划级存储根：把计划接受的条目按约定目录写成可读文件树。
//!
//! 目录约定（`classifyRule`）：`{effectiveRoot}/{sourceId}/{kind}/{itemId}{ext}`。
//! 控制存储、租约、游标仍在进程级 `--data`；本模块只按计划声明追加写出可读副本，
//! 不改统一库行为（未声明存储根的计划与今天完全一致）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use strata_model::{Body, FacetKind, Op, Target};

use crate::planner::SourcePlan;

/// 按计划声明把 ops 中的逐条内容物化为可读文件。
/// `classify: false` 或未声明存储根时不做任何事。
pub(crate) fn materialize(plan: &SourcePlan, ops: &[Op]) {
    let Some(root) = plan.storage_root.as_deref() else {
        return;
    };
    if !plan.storage_classify || root.is_empty() {
        return;
    }
    let mut identities: BTreeMap<String, String> = BTreeMap::new();
    for op in ops {
        if let Op::Ensure { identity, r#as, .. } = op {
            identities.insert(r#as.clone(), identity.clone());
        }
    }
    let source_slug = plan.source_id.as_str().split('/').next_back().unwrap_or("item");
    for op in ops {
        let Op::PutFacet {
            target: Target::Alias(alias),
            kind,
            body,
            ..
        } = op
        else {
            continue;
        };
        // 条目身份：Ensure 别名 → 稳定身份键（url:… / ext:…）。
        let identity = identities
            .get(alias)
            .cloned()
            .or_else(|| Some(alias.clone()))
            .unwrap_or_default();
        let (kind_name, ext, bytes) = match (kind, body) {
            (FacetKind::Text, Body::Text { text }) => ("text", "md", text.clone().into_bytes()),
            (FacetKind::Md, Body::Text { text }) => ("md", "md", text.clone().into_bytes()),
            (FacetKind::Struct, Body::Struct { value }) => (
                "struct",
                "json",
                serde_json::to_vec_pretty(value).unwrap_or_default(),
            ),
            _ => continue,
        };
        let item_id = sanitize(identity.strip_prefix("url:").unwrap_or(&identity));
        if item_id.is_empty() {
            continue;
        }
        let dir = PathBuf::from(root).join(source_slug).join(kind_name);
        if let Err(error) = std::fs::create_dir_all(&dir) {
            // 落盘失败不阻断采集，但必须可见：静默丢文件比报错更糟。
            tracing::warn!(source = %plan.source_id.as_str(), error = %error, "分类文件树建目录失败");
            return;
        }
        let file = dir.join(format!("{item_id}.{ext}"));
        if let Err(error) = std::fs::write(&file, bytes) {
            tracing::warn!(source = %plan.source_id.as_str(), file = %file.display(), error = %error, "分类文件写入失败");
        }
    }
}

/// 身份键 → 路径安全文件名：去掉 `..`、盘符与分隔符，仅保留字母数字与 `_-.`。
/// 超长（文件系统 255 字节上限）时截断并追加完整身份的哈希后缀，避免碰撞与静默丢失。
fn sanitize(identity: &str) -> String {
    let mut out = String::new();
    for ch in identity.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let out = out.trim_matches('.').to_string();
    if out.len() <= 200 {
        return out;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(identity, &mut hasher);
    use std::hash::Hasher;
    let suffix = format!("-{:016x}", hasher.finish());
    let keep = 200 - suffix.len();
    let mut truncated = out[..keep].to_string();
    truncated.push_str(&suffix);
    truncated
}
