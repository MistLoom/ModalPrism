//! 纯计算适配器。

mod detect;
mod feed;
mod jsonld;
mod links;
mod list;
mod page;
mod prism_detect;
mod readability;
mod smart;

pub use detect::{BasicDetector, PartDetection, detection_body};
pub use feed::{FeedFulltext, FeedItems};
pub use jsonld::JsonLd;
pub use links::LinkDiscovery;
pub use list::{HtmlList, JsonList};
pub use page::PageFields;
pub use prism_detect::PrismDetector;
pub use readability::Readability;
pub use smart::{SmartContent, SmartDiscover};
