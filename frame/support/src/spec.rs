//! Ability 声明与实现的统一入口：Manifest 与方法绑定写在一处。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    Ability, AbilityCtx, AbilityId, AbilityManifest, AbilityProvider, Dependency, Error, EventDecl,
    EventDir, Lifecycle, MethodId, MethodMeta, Result, StaticProvider,
};

type BoxCtxFut<'a> = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;
type BoxLifeFut<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
type BoxCreateFut = Pin<Box<dyn Future<Output = Result<Arc<dyn Ability>>> + Send>>;
type Handler = Arc<dyn for<'a> Fn(&'a dyn AbilityCtx, Value) -> BoxCtxFut<'a> + Send + Sync>;
type LifeHandler =
    Arc<dyn for<'a> Fn(Lifecycle, &'a dyn AbilityCtx) -> BoxLifeFut<'a> + Send + Sync>;
type CreateFn = Arc<dyn Fn(Value) -> BoxCreateFut + Send + Sync>;

#[doc(hidden)]
pub fn box_fut<'a>(
    fut: impl Future<Output = Result<Value>> + Send + 'a,
) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
    Box::pin(fut)
}

#[doc(hidden)]
pub fn box_life_fut<'a>(
    fut: impl Future<Output = Result<()>> + Send + 'a,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(fut)
}

/// 按 config 每次 `create` 新实例。有状态能力必须走此路径，禁止跨 load 复用 Arc。
pub struct FactoryProvider {
    manifest: AbilityManifest,
    create: CreateFn,
}

impl FactoryProvider {
    pub fn new<F, Fut>(manifest: AbilityManifest, f: F) -> Arc<dyn AbilityProvider>
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Arc<dyn Ability>>> + Send + 'static,
    {
        Arc::new(Self {
            manifest,
            create: Arc::new(move |config| Box::pin(f(config))),
        })
    }
}

#[async_trait]
impl AbilityProvider for FactoryProvider {
    fn manifest(&self) -> AbilityManifest {
        self.manifest.clone()
    }

    async fn create(&self, config: Value) -> Result<Arc<dyn Ability>> {
        (self.create)(config).await
    }
}

/// 信息声明 + 方法实现。`provider()` 由此产出。
pub struct AbilitySpec {
    id: String,
    depends: Vec<Dependency>,
    stateful: bool,
    config_schema: Value,
    methods: Vec<MethodMeta>,
    events: Vec<EventDecl>,
    handlers: HashMap<String, Handler>,
    lifecycle: Option<LifeHandler>,
}

impl AbilitySpec {
    pub fn try_new(id: impl Into<String>) -> Result<Self> {
        let id = id.into();
        AbilityId::new(&id).map_err(|e| Error::invalid_args(e.to_string()))?;
        Ok(Self {
            id,
            depends: Vec::new(),
            stateful: false,
            config_schema: Value::Null,
            methods: Vec::new(),
            events: Vec::new(),
            handlers: HashMap::new(),
            lifecycle: None,
        })
    }

    pub fn must(id: &'static str) -> Self {
        Self::try_new(id).expect("Ability id")
    }

    /// 从已有 Manifest 出发（方法体由自定义 Ability / factory 提供）。
    pub fn from_manifest(manifest: AbilityManifest) -> Self {
        Self {
            id: manifest.id.to_string(),
            depends: manifest.depends,
            stateful: manifest.stateful,
            config_schema: manifest.config_schema,
            methods: manifest.methods,
            events: manifest.events,
            handlers: HashMap::new(),
            lifecycle: None,
        }
    }

    pub fn stateful(mut self) -> Self {
        self.stateful = true;
        self
    }

    pub fn config_schema(mut self, schema: Value) -> Self {
        self.config_schema = schema;
        self
    }

    pub fn depends(mut self, dep: Dependency) -> Self {
        self.depends.push(dep);
        self
    }

    pub fn depends_id(mut self, id: &str) -> Self {
        let aid = AbilityId::new(id).expect("depends id");
        self.depends.push(Dependency::required(aid));
        self
    }

    pub fn depends_optional(mut self, id: &str) -> Self {
        let aid = AbilityId::new(id).expect("depends id");
        self.depends.push(Dependency::optional(aid));
        self
    }

    pub fn depends_alias(mut self, id: &str, alias: &str) -> Self {
        let aid = AbilityId::new(id).expect("depends id");
        self.depends.push(Dependency {
            id: aid,
            alias: Some(alias.to_string()),
            optional: false,
        });
        self
    }

    pub fn event_emit(mut self, name: impl Into<String>) -> Self {
        self.events.push(EventDecl {
            name: name.into(),
            dir: EventDir::Emit,
            description: String::new(),
            payload_schema: Value::Null,
        });
        self
    }

    pub fn event_listen(mut self, name: impl Into<String>) -> Self {
        self.events.push(EventDecl {
            name: name.into(),
            dir: EventDir::Listen,
            description: String::new(),
            payload_schema: Value::Null,
        });
        self
    }

    /// 只声明方法，实现放在自定义 `Ability`。
    pub fn method(mut self, id: &str, description: &str, input_schema: Value) -> Self {
        self.methods
            .push(MethodMeta::new(id, description, input_schema));
        self
    }

    pub fn on<F>(mut self, id: &str, description: &str, input_schema: Value, f: F) -> Self
    where
        F: for<'a> Fn(&'a dyn AbilityCtx, Value) -> BoxCtxFut<'a> + Send + Sync + 'static,
    {
        let mid = MethodId::new(id).expect("method id");
        self.methods.push(MethodMeta {
            id: mid,
            description: description.into(),
            input_schema,
            output_schema: None,
        });
        self.handlers.insert(id.to_string(), Arc::new(f));
        self
    }

    pub fn on_lifecycle<F>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(Lifecycle, &'a dyn AbilityCtx) -> BoxLifeFut<'a> + Send + Sync + 'static,
    {
        self.lifecycle = Some(Arc::new(f));
        self
    }

    pub fn manifest(&self) -> AbilityManifest {
        AbilityManifest {
            id: AbilityId::new(&self.id).expect("id"),
            depends: self.depends.clone(),
            stateful: self.stateful,
            config_schema: self.config_schema.clone(),
            methods: self.methods.clone(),
            events: self.events.clone(),
        }
    }

    fn into_mapped(self) -> MappedAbility {
        MappedAbility {
            manifest: self.manifest(),
            handlers: self.handlers,
            lifecycle: self.lifecycle,
        }
    }

    /// 无状态：共享同一实例。有状态：每次 create 新建，避免 unload 后残留。
    pub fn into_provider(self) -> Arc<dyn AbilityProvider> {
        if self.stateful {
            let manifest = self.manifest();
            let handlers = self.handlers.clone();
            let lifecycle = self.lifecycle.clone();
            FactoryProvider::new(manifest.clone(), move |_config| {
                let ability = MappedAbility {
                    manifest: manifest.clone(),
                    handlers: handlers.clone(),
                    lifecycle: lifecycle.clone(),
                };
                async move { Ok(Arc::new(ability) as Arc<dyn Ability>) }
            })
        } else {
            Arc::new(StaticProvider::new(Arc::new(self.into_mapped())))
        }
    }

    /// 自定义实例化（读 config、持有资源）。有状态能力的标准入口。
    pub fn into_factory<F, Fut>(self, f: F) -> Arc<dyn AbilityProvider>
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Arc<dyn Ability>>> + Send + 'static,
    {
        FactoryProvider::new(self.manifest(), f)
    }
}

struct MappedAbility {
    manifest: AbilityManifest,
    handlers: HashMap<String, Handler>,
    lifecycle: Option<LifeHandler>,
}

#[async_trait]
impl Ability for MappedAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn on_lifecycle(&self, phase: Lifecycle, ctx: &dyn AbilityCtx) -> Result<()> {
        if let Some(h) = &self.lifecycle {
            h(phase, ctx).await
        } else {
            Ok(())
        }
    }

    async fn invoke(&self, method: &MethodId, ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        let h = self
            .handlers
            .get(method.as_str())
            .ok_or_else(|| Error::not_found(method))?;
        h(ctx, args).await
    }
}
