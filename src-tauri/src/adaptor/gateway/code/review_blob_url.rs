//! Review image / binary 用 custom URI の URL provider。

use crate::domain::code::{ReviewBlobSide, ReviewBlobUrlParams, ReviewBlobUrlProvider};

pub struct ReviewBlobUrlGateway;

impl ReviewBlobUrlProvider for ReviewBlobUrlGateway {
    fn url(&self, params: &ReviewBlobUrlParams) -> String {
        let mut url =
            url::Url::parse("review-blob://localhost/blob").expect("valid review blob URL");
        let side = match params.side {
            ReviewBlobSide::Original => "original",
            ReviewBlobSide::Modified => "modified",
        };
        url.query_pairs_mut()
            .append_pair("worktree", &params.worktree_path)
            .append_pair("path", &params.path)
            .append_pair("side", side)
            .append_pair("section", &params.section)
            .append_pair("base", &params.base)
            .append_pair("version", &params.version.to_string());
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_url_refs_from_port_params() {
        let url = ReviewBlobUrlGateway.url(&ReviewBlobUrlParams {
            worktree_path: "/repo".to_string(),
            path: "src/image.png".to_string(),
            side: ReviewBlobSide::Original,
            section: "changes".to_string(),
            base: "head".to_string(),
            version: 9,
        });

        assert!(url.starts_with("review-blob://"));
        assert!(url.contains("path=src%2Fimage.png"));
        assert!(url.contains("side=original"));
        assert!(url.contains("version=9"));
    }
}
