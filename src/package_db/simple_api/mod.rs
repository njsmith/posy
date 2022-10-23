mod fetch;
mod html;
mod project_info;

pub use fetch::fetch_simple_api;
use html::parse_html;
pub use project_info::{pack_by_version, ArtifactInfo, ProjectInfo};
