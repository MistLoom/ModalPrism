//! 内置适配器的程序侧声明。
//!
//! 适配器实现在代码里登记（`adapters::Registry::builtins`），使用方式却在配置里按 id
//! 引用：信源管道写 `adapter/smart-discover`，校验要能在模块表里找到它。两边只要缺一边，
//! 声明期就会把运行期确实存在的适配器判成悬空引用。
//!
//! 因此这里带上仓库里的声明文本，加载时补进模块表：盘上有同名声明以盘上为准（用户改写
//! 立即生效），没有则用这一份兜底，老部署不必等配置迁移也能引用内置适配器。

pub struct BuiltinAdapter {
    pub id: &'static str,
    pub source: &'static str,
}

/// 全部内置适配器的声明文本，与 `adapters::Registry::builtins` 的登记一一对应。
pub fn builtin_adapters() -> [BuiltinAdapter; 10] {
    [
        BuiltinAdapter {
            id: "adapter/feed-items",
            source: include_str!("../../../../config/adapters/feed-items.yaml"),
        },
        BuiltinAdapter {
            id: "adapter/feed-fulltext",
            source: include_str!("../../../../config/adapters/feed-fulltext.yaml"),
        },
        BuiltinAdapter {
            id: "adapter/html-list",
            source: include_str!("../../../../config/adapters/html-list.yaml"),
        },
        BuiltinAdapter {
            id: "adapter/json-list",
            source: include_str!("../../../../config/adapters/json-list.yaml"),
        },
        BuiltinAdapter {
            id: "adapter/jsonld-struct",
            source: include_str!("../../../../config/adapters/jsonld-struct.yaml"),
        },
        BuiltinAdapter {
            id: "adapter/link-discovery",
            source: include_str!("../../../../config/adapters/link-discovery.yaml"),
        },
        BuiltinAdapter {
            id: "adapter/page-fields",
            source: include_str!("../../../../config/adapters/page-fields.yaml"),
        },
        BuiltinAdapter {
            id: "adapter/readability-md",
            source: include_str!("../../../../config/adapters/readability-md.yaml"),
        },
        BuiltinAdapter {
            id: "adapter/smart-content",
            source: include_str!("../../../../config/adapters/smart-content.yaml"),
        },
        BuiltinAdapter {
            id: "adapter/smart-discover",
            source: include_str!("../../../../config/adapters/smart-discover.yaml"),
        },
    ]
}
