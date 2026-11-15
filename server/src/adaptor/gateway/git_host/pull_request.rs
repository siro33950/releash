use serde::Deserialize;

use crate::domain::git_host::{
    ActionsJobId, Check, CheckBucket, GitHostError, PrComment, PrCommentAuthor, PrInfo, PrState,
};

pub(super) fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, GitHostError> {
    serde_json::from_str(text)
        .map_err(|error| GitHostError::External(format!("GitHub response is invalid: {error}")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Detail {
    pub number: u64,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
    pub title: String,
    pub body: String,
}
impl Detail {
    pub fn info(&self) -> Result<PrInfo, GitHostError> {
        let state = match self.state.as_str() {
            "OPEN" => PrState::Open,
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            value => return Err(GitHostError::External(format!("Unknown PR state: {value}"))),
        };
        Ok(PrInfo {
            number: self.number,
            url: self.url.clone(),
            state,
            draft: self.is_draft,
        })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GhCheck {
    name: String,
    bucket: String,
    started_at: Option<String>,
    completed_at: Option<String>,
    link: String,
}
impl GhCheck {
    pub fn into_check(self) -> Result<Check, GitHostError> {
        let bucket = match self.bucket.as_str() {
            "fail" => CheckBucket::Fail,
            "cancel" => CheckBucket::Cancel,
            "pending" => CheckBucket::Pending,
            "skipping" => CheckBucket::Skipping,
            "pass" => CheckBucket::Pass,
            value => {
                return Err(GitHostError::External(format!(
                    "Unknown check bucket: {value}"
                )))
            }
        };
        let job = self
            .link
            .strip_prefix("https://github.com/")
            .and_then(|path| path.split_once("/actions/runs/"))
            .and_then(|(_, path)| path.split_once("/job/"))
            .and_then(|(_, id)| id.parse().ok())
            .map(ActionsJobId);
        Ok(Check {
            name: self.name,
            bucket,
            started_at: optional_time(self.started_at.as_deref())?,
            completed_at: optional_time(self.completed_at.as_deref())?,
            job,
        })
    }
}
fn optional_time(value: Option<&str>) -> Result<Option<u64>, GitHostError> {
    value
        .filter(|value| !matches!(*value, "" | "0001-01-01T00:00:00Z"))
        .map(timestamp)
        .transpose()
}
fn timestamp(value: &str) -> Result<u64, GitHostError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|date| u64::try_from(date.timestamp()).ok())
        .ok_or_else(|| GitHostError::External(format!("Invalid GitHub timestamp: {value}")))
}
#[derive(Deserialize)]
struct User {
    login: String,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
pub(super) struct Comment {
    user: User,
    body: String,
    created_at: String,
}
impl Comment {
    pub fn into_comment(self) -> Result<PrComment, GitHostError> {
        Ok(PrComment {
            author: PrCommentAuthor {
                login: self.user.login,
                is_bot: self.user.kind == "Bot",
            },
            body: self.body,
            written_at: timestamp(&self.created_at)?,
        })
    }
}

#[derive(Deserialize)]
pub(super) struct Review {
    user: User,
    body: String,
    submitted_at: Option<String>,
}
impl Review {
    pub fn into_comment(self) -> Result<Option<PrComment>, GitHostError> {
        let Some(time) = self.submitted_at else {
            return Ok(None);
        };
        Ok(Some(PrComment {
            author: PrCommentAuthor {
                login: self.user.login,
                is_bot: self.user.kind == "Bot",
            },
            body: self.body,
            written_at: timestamp(&time)?,
        }))
    }
}
