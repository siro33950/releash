use crate::adaptor::presenter::client as wire;
use crate::domain::git_host::value_objects::pr::PrClassification;
use crate::domain::git_host::{BranchPullRequest, CheckBucket, PullRequest};
use crate::usecase::fetched::Fetched;

pub(super) fn snapshot(value: &Fetched<BranchPullRequest>) -> wire::PullRequestSnapshot {
    use wire::pull_request_snapshot::BranchPullRequest as B;
    wire::PullRequestSnapshot {
        read_error: value.error.as_ref().map(ToString::to_string),
        branch_pull_request: value.value.as_ref().map(|pr| match pr {
            BranchPullRequest::Absent => B::Absent(wire::Unit {}),
            BranchPullRequest::Present(pr) => B::Present(Box::new(view(pr))),
        }),
    }
}
fn view(pr: &PullRequest) -> wire::PullRequestView {
    let checks = &pr.checks;
    wire::PullRequestView {
        number: pr.info.number,
        title: pr.title.clone(),
        body: pr.body.clone(),
        url: pr.info.url.clone(),
        state: match pr.info.classification() {
            PrClassification::Open => wire::PullRequestState::Open,
            PrClassification::Draft => wire::PullRequestState::Draft,
            PrClassification::Merged => wire::PullRequestState::Merged,
            PrClassification::Closed => wire::PullRequestState::Closed,
        } as i32,
        can_create_new: pr.info.allows_new_pull_request(),
        checks: Some(wire::PullRequestChecks {
            failed: checks.count(CheckBucket::Fail) as u32,
            cancelled: checks.count(CheckBucket::Cancel) as u32,
            pending: checks.count(CheckBucket::Pending) as u32,
            skipped: checks.count(CheckBucket::Skipping) as u32,
            passed: checks.count(CheckBucket::Pass) as u32,
            passed_names: checks
                .passed_names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            skipped_names: checks
                .skipped_names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            listed: checks
                .listed()
                .into_iter()
                .map(|check| wire::PullRequestCheck {
                    name: check.name.clone(),
                    bucket: match check.bucket {
                        CheckBucket::Fail => wire::PullRequestCheckBucket::Fail,
                        CheckBucket::Cancel => wire::PullRequestCheckBucket::Cancel,
                        CheckBucket::Pending => wire::PullRequestCheckBucket::Pending,
                        CheckBucket::Skipping | CheckBucket::Pass => {
                            wire::PullRequestCheckBucket::Unspecified
                        }
                    } as i32,
                    failure_seconds: check.failure_seconds(),
                    started_at: check.running_since(),
                    log_job_id: check.log_job().map(|job| job.0),
                })
                .collect(),
        }),
        comments: pr
            .conversation
            .entries()
            .iter()
            .map(|entry| wire::PullRequestComment {
                author: entry.author.login.clone(),
                bot: entry.author.is_bot,
                written_at: entry.written_at,
                body: entry.body.clone(),
            })
            .collect(),
    }
}
