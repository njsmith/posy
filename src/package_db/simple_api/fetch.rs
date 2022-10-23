use super::super::http::{Http, CacheMode};
use super::project_info::ProjectInfo;
use crate::prelude::*;

use http::Request;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    etag: Option<String>,
    last_modified: Option<String>,
    content_type: String,
    body: String,
}

pub fn fetch_simple_api(http: &Http, url: &Url) -> Result<ProjectInfo> {
    let request = Request::builder()
        .uri(url.as_str())
        .header("Cache-Control", "max-age=0")
        .body(())?;

    let response = http.request(request, CacheMode::Default)?;
    let url = response.extensions().get::<Url>().unwrap().to_owned();
    let content_type = if let Some(value) = response.headers().get("Content-Type") {
        value.to_str()?
    } else {
        "text/html"
    }.to_owned();

    Ok(super::parse_html(
        &url,
        &content_type,
        response.into_body(),
    )?)
}
