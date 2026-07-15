#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepresentativeStatus {
    Running,
    Failed,
    Error,
    Waiting,
    Interrupted,
    Aborted,
    Completed,
    Queued,
}

impl RepresentativeStatus {
    pub(crate) fn priority(self) -> u8 {
        match self {
            Self::Running => 1,
            Self::Failed => 2,
            Self::Error => 3,
            Self::Waiting => 4,
            Self::Interrupted => 5,
            Self::Aborted => 6,
            Self::Completed => 7,
            Self::Queued => 8,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Error => "error",
            Self::Waiting => "waiting",
            Self::Interrupted => "interrupted",
            Self::Aborted => "aborted",
            Self::Completed => "completed",
            Self::Queued => "queued",
        }
    }

    pub(crate) fn from_status_str(status: &str) -> Self {
        match status {
            "running" => Self::Running,
            "failed" => Self::Failed,
            "error" => Self::Error,
            "waiting" | "waiting_approval" => Self::Waiting,
            "interrupted" => Self::Interrupted,
            "aborted" => Self::Aborted,
            "completed" | "succeeded" => Self::Completed,
            "queued" | "pending" => Self::Queued,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepProgress {
    Failed,
    WaitingApproval,
    Running,
    Aborted,
    Completed,
    Queued,
}

impl StepProgress {
    pub(crate) fn from_status_str(status: &str) -> Self {
        match status {
            "failed" => Self::Failed,
            "waiting_approval" | "waiting" => Self::WaitingApproval,
            "running" => Self::Running,
            "aborted" | "interrupted" => Self::Aborted,
            "completed" | "succeeded" => Self::Completed,
            "pending" | "queued" => Self::Queued,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionActivity {
    Running,
    Waiting,
    Done,
    Error,
}

pub(crate) fn session_result(
    step: StepProgress,
    activity: SessionActivity,
) -> RepresentativeStatus {
    match activity {
        SessionActivity::Running => RepresentativeStatus::Running,
        SessionActivity::Waiting => match step {
            StepProgress::Failed => RepresentativeStatus::Failed,
            StepProgress::WaitingApproval
            | StepProgress::Running
            | StepProgress::Aborted
            | StepProgress::Completed
            | StepProgress::Queued => RepresentativeStatus::Waiting,
        },
        SessionActivity::Done => match step {
            StepProgress::Failed => RepresentativeStatus::Failed,
            StepProgress::WaitingApproval => RepresentativeStatus::Waiting,
            StepProgress::Running => RepresentativeStatus::Running,
            StepProgress::Aborted => RepresentativeStatus::Aborted,
            StepProgress::Completed => RepresentativeStatus::Completed,
            StepProgress::Queued => RepresentativeStatus::Queued,
        },
        SessionActivity::Error => match step {
            StepProgress::Failed => RepresentativeStatus::Failed,
            StepProgress::WaitingApproval
            | StepProgress::Running
            | StepProgress::Aborted
            | StepProgress::Completed
            | StepProgress::Queued => RepresentativeStatus::Error,
        },
    }
}

pub(crate) fn aggregate_representative_statuses(
    statuses: impl IntoIterator<Item = RepresentativeStatus>,
) -> Option<RepresentativeStatus> {
    statuses.into_iter().min_by_key(|status| status.priority())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_matches_spec_order() {
        assert_eq!(
            [
                RepresentativeStatus::Running,
                RepresentativeStatus::Failed,
                RepresentativeStatus::Error,
                RepresentativeStatus::Waiting,
                RepresentativeStatus::Interrupted,
                RepresentativeStatus::Aborted,
                RepresentativeStatus::Completed,
                RepresentativeStatus::Queued,
            ]
            .map(RepresentativeStatus::priority),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn session_result_matches_step_by_agent_cross_table() {
        use RepresentativeStatus as R;
        use SessionActivity::{Done, Error, Running, Waiting};
        use StepProgress as S;

        let cases = [
            (S::Failed, Running, R::Running),
            (S::WaitingApproval, Running, R::Running),
            (S::Running, Running, R::Running),
            (S::Aborted, Running, R::Running),
            (S::Completed, Running, R::Running),
            (S::Queued, Running, R::Running),
            (S::Failed, Waiting, R::Failed),
            (S::WaitingApproval, Waiting, R::Waiting),
            (S::Running, Waiting, R::Waiting),
            (S::Aborted, Waiting, R::Waiting),
            (S::Completed, Waiting, R::Waiting),
            (S::Queued, Waiting, R::Waiting),
            (S::Failed, Done, R::Failed),
            (S::WaitingApproval, Done, R::Waiting),
            (S::Running, Done, R::Running),
            (S::Aborted, Done, R::Aborted),
            (S::Completed, Done, R::Completed),
            (S::Queued, Done, R::Queued),
            (S::Failed, Error, R::Failed),
            (S::WaitingApproval, Error, R::Error),
            (S::Running, Error, R::Error),
            (S::Aborted, Error, R::Error),
            (S::Completed, Error, R::Error),
            (S::Queued, Error, R::Error),
        ];

        for (step, activity, expected) in cases {
            assert_eq!(session_result(step, activity), expected);
        }
    }

    #[test]
    fn aggregate_returns_strongest_priority() {
        use RepresentativeStatus as R;
        for (statuses, expected) in [
            (vec![R::Running, R::Waiting, R::Completed], R::Running),
            (vec![R::Failed, R::Waiting, R::Completed], R::Failed),
            (vec![R::Error, R::Waiting, R::Queued], R::Error),
            (vec![R::Waiting, R::Completed, R::Queued], R::Waiting),
            (
                vec![R::Interrupted, R::Aborted, R::Completed],
                R::Interrupted,
            ),
            (vec![R::Aborted, R::Completed, R::Queued], R::Aborted),
            (vec![R::Completed, R::Queued], R::Completed),
            (vec![R::Queued, R::Queued], R::Queued),
        ] {
            assert_eq!(aggregate_representative_statuses(statuses), Some(expected));
        }
    }

    #[test]
    fn aggregate_single_and_empty_inputs() {
        assert_eq!(
            aggregate_representative_statuses([RepresentativeStatus::Waiting]),
            Some(RepresentativeStatus::Waiting)
        );
        assert_eq!(aggregate_representative_statuses(std::iter::empty()), None);
    }
}
