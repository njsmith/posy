mod project_info;
mod html;
mod fetch;

use html::parse_html;
pub use project_info::{ProjectInfo, ArtifactInfo, pack_by_version};
pub use fetch::fetch_simple_api;
