//! 适配器与检测器登记。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::builtin::{
    BasicDetector, FeedFulltext, FeedItems, HtmlList, JsonLd, JsonList, LinkDiscovery, PageFields,
    PrismDetector, Readability, SmartContent, SmartDiscover,
};
use crate::error::AdapterError;
use crate::model::AdapterRole;
use crate::prism::Prism;
use crate::traits::{ContentAdapter, Detector, SearchAdapter};

pub struct Registry {
    search: BTreeMap<String, Arc<dyn SearchAdapter>>,
    content: BTreeMap<String, Arc<dyn ContentAdapter>>,
    detectors: BTreeMap<String, Arc<dyn Detector>>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            search: BTreeMap::new(),
            content: BTreeMap::new(),
            detectors: BTreeMap::new(),
        }
    }

    pub fn builtins() -> Self {
        Self::builtins_with(None)
    }

    /// 内置登记。注入识别引擎时额外登记 `detector/prism`，未注入则只有基础检测。
    /// 智能适配器总是登记：没有识别器时它们按媒体类型启发，有识别器时按区域切分。
    pub fn builtins_with(prism: Option<Arc<dyn Prism>>) -> Self {
        let mut registry = Self::with_base(prism.clone());
        if let Some(prism) = prism {
            registry.register_detector(Arc::new(PrismDetector::new(prism)));
        }
        registry
    }

    fn with_base(prism: Option<Arc<dyn Prism>>) -> Self {
        let mut registry = Self::new();
        registry
            .register_search(Arc::new(FeedItems::new()))
            .expect("内置搜索适配器");
        registry
            .register_search(Arc::new(HtmlList::new()))
            .expect("内置搜索适配器");
        registry
            .register_search(Arc::new(JsonList::new()))
            .expect("内置搜索适配器");
        registry
            .register_search(Arc::new(LinkDiscovery::new()))
            .expect("内置搜索适配器");
        registry
            .register_content(Arc::new(FeedFulltext::new()))
            .expect("内置内容适配器");
        registry
            .register_content(Arc::new(Readability::new()))
            .expect("内置内容适配器");
        registry
            .register_content(Arc::new(PageFields::new()))
            .expect("内置内容适配器");
        registry
            .register_content(Arc::new(JsonLd::new()))
            .expect("内置内容适配器");
        registry
            .register_search(Arc::new(SmartDiscover::new(prism.clone())))
            .expect("内置智能适配器");
        registry
            .register_content(Arc::new(SmartContent::new(prism)))
            .expect("内置智能适配器");
        registry.register_detector(Arc::new(BasicDetector));
        registry
    }

    pub fn register_search(&mut self, adapter: Arc<dyn SearchAdapter>) -> Result<(), AdapterError> {
        let meta = adapter.meta();
        if meta.role != AdapterRole::Search {
            return Err(AdapterError::config(format!("{} 不是搜索适配器", meta.id)));
        }
        self.search.insert(meta.id.clone(), adapter);
        Ok(())
    }

    pub fn register_content(
        &mut self,
        adapter: Arc<dyn ContentAdapter>,
    ) -> Result<(), AdapterError> {
        let meta = adapter.meta();
        if meta.role != AdapterRole::Content {
            return Err(AdapterError::config(format!("{} 不是内容适配器", meta.id)));
        }
        self.content.insert(meta.id.clone(), adapter);
        Ok(())
    }

    pub fn register_detector(&mut self, detector: Arc<dyn Detector>) {
        self.detectors.insert(detector.id().to_string(), detector);
    }

    pub fn search(&self, id: &str) -> Option<Arc<dyn SearchAdapter>> {
        self.search.get(id).cloned()
    }

    pub fn content(&self, id: &str) -> Option<Arc<dyn ContentAdapter>> {
        self.content.get(id).cloned()
    }

    pub fn detector(&self, id: &str) -> Option<Arc<dyn Detector>> {
        self.detectors.get(id).cloned()
    }

    /// 已登记的适配器 id（搜索与内容合一）。配置侧的声明清单要与这一份对齐。
    pub fn adapter_ids(&self) -> Vec<String> {
        self.search.keys().chain(self.content.keys()).cloned().collect()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Registry;

    /// 代码登记的内置适配器与配置声明必须一一对应。
    ///
    /// 信源管道按 id 引用适配器，配置校验只认模块表里的声明：新增内置适配器而不给声明，
    /// 它就只能靠默认管道兜着，用户显式引用时会得到「引用不存在」。
    #[test]
    fn builtin_adapters_are_declared_in_config() {
        let mut registered = Registry::builtins().adapter_ids();
        registered.sort();
        let mut declared: Vec<String> = config::builtin_adapters()
            .iter()
            .map(|item| item.id.to_string())
            .collect();
        declared.sort();
        assert_eq!(
            registered, declared,
            "内置适配器的代码登记与 config::builtin_adapters 清单不一致"
        );
    }

    /// 每份程序侧声明都要能解析且 id 自洽，否则加载时会被判成坏文档。
    #[test]
    fn builtin_adapter_declarations_are_self_consistent() {
        for item in config::builtin_adapters() {
            let doc = config::parse_yaml("builtins.yaml", item.source)
                .unwrap_or_else(|error| panic!("{} 声明无法解析: {error}", item.id));
            assert_eq!(doc["id"], item.id, "{} 声明的 id 与清单不符", item.id);
            assert_eq!(doc["kind"], "adapter", "{} 声明的 kind 不对", item.id);
            assert_eq!(
                doc["implementation"],
                format!("builtin:{}", item.id.trim_start_matches("adapter/")),
                "{} 的实现名要与登记名同源",
                item.id
            );
        }
    }
}
