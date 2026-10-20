//! 适配器与检测器登记。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::builtin::{
    BasicDetector, FeedFulltext, FeedItems, HtmlList, JsonLd, JsonList, LinkDiscovery, Readability,
};
use crate::error::AdapterError;
use crate::model::AdapterRole;
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
            .register_content(Arc::new(JsonLd::new()))
            .expect("内置内容适配器");
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
