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
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
