//! 适配器契约、路由和纯计算适配器。
//!
//! 适配器返回发现或内容草稿。调用方把 [`run`] 的变更集交给 Strata，本 crate 不写入。

mod builtin;
mod compile;
mod error;
mod model;
mod registry;
mod route;
mod run;
mod textutil;
mod traits;

pub use compile::{
    Admission, Batch, CompileInput, Compiled, Emitted, EmittedDiscovery, FollowRequest,
    IdentityHit, IdentityView, MapIdentities, compile,
};
pub use error::AdapterError;
pub use model::{
    AdapterInput, AdapterMeta, AdapterRole, ContentOutput, Declared, Detection, DiscoverOutput,
    Discovery, DiscoveryHints, DraftBody, FacetDraft, Feedback, Follow, InputSelector, LinkDraft,
    LinkTarget, OutputDecl, Probe, Quality, ReadonlyView, RegionHit, RenderMode, SourceView,
};
pub use registry::Registry;
pub use route::{Candidate, Rule, Step, parse_pipeline};
pub use run::{
    AdapterFeedback, DetectorChoice, PartBytes, RunOutput, RunRequest, empty_identities, run,
    source_view,
};
pub use traits::{ContentAdapter, Detector, SearchAdapter};

pub use builtin::{
    BasicDetector, FeedFulltext, FeedItems, HtmlList, JsonLd, JsonList, LinkDiscovery, PageFields,
    Readability,
};
