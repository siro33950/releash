use crate::domain::code::ReviewBase;

use crate::domain::workflow::FacetKind;
use crate::usecase::notion::usecase::NotionTaskListRequest;
use crate::usecase::state_subscription::{SubscriptionError, SubscriptionTarget};

impl SubscriptionTarget {
    pub fn parse(raw: &str) -> Result<Self, SubscriptionError> {
        let (name, mut rest) = raw.split_once(':').unwrap_or((raw, ""));
        let mut args = Vec::new();
        while !rest.is_empty() {
            let (length, tail) = rest.split_once(':').ok_or(SubscriptionError::InvalidId)?;
            let length: usize = length.parse().map_err(|_| SubscriptionError::InvalidId)?;
            let (arg, tail) = tail
                .split_at_checked(length)
                .ok_or(SubscriptionError::InvalidId)?;
            args.push(arg);
            rest = tail;
        }
        let target = Self::from_parts(name, &args)?;
        if target.to_string() != raw {
            return Err(SubscriptionError::InvalidId);
        }
        Ok(target)
    }

    pub fn from_parts(name: &str, args: &[&str]) -> Result<Self, SubscriptionError> {
        if args
            .iter()
            .any(|arg| arg.trim().is_empty() || arg.contains('\0'))
        {
            return Err(SubscriptionError::InvalidId);
        }
        if matches!(name, "directory" | "file-tab" | "file-threads") {
            let Some(file) = args.get(1) else {
                return Err(SubscriptionError::InvalidId);
            };
            if !crate::domain::code::services::directory::worktree_path_is_visible(file) {
                return Err(SubscriptionError::InvalidId);
            }
        }
        let target = match (name, args) {
            ("daemon-info", []) => Ok(Self::DaemonInfo),
            ("workspace-terminal", [path, id]) => {
                crate::domain::terminal_surface::TerminalSurfaceOwner::workspace_terminal(
                    crate::domain::workspace_tree::WorkspaceIdentity::new(*path),
                    *id,
                )
                .map(Self::Terminal)
                .map_err(|_| SubscriptionError::InvalidId)
            }
            ("terminal", [path]) => {
                crate::domain::terminal_surface::TerminalSurfaceOwner::workspace(
                    crate::domain::workspace_tree::WorkspaceIdentity::new(*path),
                )
                .map(Self::Terminal)
                .map_err(|_| SubscriptionError::InvalidId)
            }
            ("terminal", [path, id]) => {
                crate::domain::terminal_surface::TerminalSurfaceOwner::session(
                    crate::domain::workspace_tree::WorkspaceIdentity::new(*path),
                    *id,
                )
                .map(Self::Terminal)
                .map_err(|_| SubscriptionError::InvalidId)
            }
            ("repository-group-state", [path]) => Ok(Self::RepositoryGroupState((*path).into())),
            ("workspaces", []) => Ok(Self::Workspaces),
            ("node-detail", [path, id]) => Ok(Self::NodeDetail((*path).into(), (*id).into())),
            ("session-tab", [id]) => Ok(Self::SessionTab((*id).into())),
            ("session-history", [path, count]) => {
                let parsed: usize = count.parse().map_err(|_| SubscriptionError::InvalidId)?;
                if parsed == 0 || parsed.to_string() != *count {
                    return Err(SubscriptionError::InvalidId);
                }
                Ok(Self::SessionHistory((*path).into(), parsed))
            }
            ("providers", []) => Ok(Self::Providers),
            ("branches", [path]) => Ok(Self::Branches((*path).into(), None)),
            ("branches", [path, excluded]) => {
                Ok(Self::Branches((*path).into(), Some((*excluded).into())))
            }
            ("available-branches", [path]) => Ok(Self::AvailableBranches((*path).into())),
            ("issues", [path]) => Ok(Self::Issues((*path).into())),
            ("issues", [path, filters @ ..]) => {
                let mut filter = crate::domain::git_host::IssueFilter::default();
                for value in filters {
                    match value.split_once('=') {
                        Some(("label", value)) if !value.is_empty() => {
                            filter.labels.push(value.into())
                        }
                        Some(("milestone", value))
                            if !value.is_empty() && filter.milestone.is_none() =>
                        {
                            filter.milestone = Some(value.into())
                        }
                        _ => return Err(SubscriptionError::InvalidId),
                    }
                }
                filter.labels.sort();
                filter.labels.dedup();
                Ok(Self::FilteredIssues((*path).into(), filter))
            }
            ("notion-tasks", [path, count, filters @ ..]) => {
                parse_notion_tasks(path, count, filters)
            }
            ("workspace-state", [name, path]) => {
                Ok(Self::WorkspaceState((*name).into(), (*path).into()))
            }
            ("directory", [path, directory]) => {
                Ok(Self::Directory((*path).into(), (*directory).into()))
            }
            ("filtered-files", [path, filters @ ..]) => Ok(Self::FilteredFiles(
                (*path).into(),
                parse_file_filter(filters)?,
            )),
            ("file-tab", [path, file, base]) => Ok(Self::FileTab(
                (*path).into(),
                (*file).into(),
                ReviewBase::parse(base).map_err(|_| SubscriptionError::InvalidId)?,
            )),
            ("file-threads", [path, file]) => Ok(Self::FileThreads((*path).into(), (*file).into())),
            ("review-threads", [name]) => Ok(Self::ReviewThreads((*name).into())),
            ("workflow-execution", [id]) => Ok(Self::WorkflowExecution((*id).into())),
            ("node-artifact", [id]) => Ok(Self::NodeArtifact((*id).into())),
            ("workflow-output", [id, node]) => {
                Ok(Self::WorkflowOutput((*id).into(), (*node).into()))
            }
            ("review-session-threads", [id, filters @ ..]) => Ok(Self::ReviewSessionThreads(
                (*id).into(),
                parse_session_review_filters(filters)?,
            )),
            ("review-worktree-threads", [path, filters @ ..]) => Ok(Self::ReviewWorktreeThreads(
                (*path).into(),
                parse_worktree_review_filters(filters)?,
            )),
            ("review-session-thread", [id, thread]) => {
                Ok(Self::ReviewSessionThread((*id).into(), (*thread).into()))
            }
            ("review-session-thread-history", [id, thread]) => Ok(
                Self::ReviewSessionThreadHistory((*id).into(), (*thread).into()),
            ),
            ("workflows", []) => Ok(Self::Workflows),
            ("workflow", [name]) => Ok(Self::Workflow((*name).into())),
            ("facets", [kind]) => Ok(Self::Facets(facet_kind(kind)?)),
            ("diagnostics", []) => Ok(Self::Diagnostics),
            ("desktop-settings", []) => Ok(Self::DesktopSettings),
            ("notion-config", [path]) => Ok(Self::NotionConfig((*path).into())),
            ("provider-availability", []) => Ok(Self::ProviderAvailability),
            ("external-editor", []) => Ok(Self::ExternalEditor),
            ("releash-base", [path]) => Ok(Self::ReleashBase((*path).into())),
            ("terminal-startup-command", []) => Ok(Self::TerminalStartupCommand),
            ("workflow-config", []) => Ok(Self::WorkflowConfig),
            ("provider-usage", []) => Ok(Self::ProviderUsage),
            ("provider-hook-health", []) => Ok(Self::ProviderHookHealth),
            ("execution-overview", []) => Ok(Self::ExecutionOverview),
            _ => Err(SubscriptionError::UnknownTarget),
        }?;
        Ok(target)
    }
}

impl SubscriptionTarget {
    pub fn parts(&self) -> (&'static str, Vec<String>) {
        match self {
            Self::DaemonInfo => ("daemon-info", vec![]),
            Self::Terminal(owner) => match owner {
                crate::domain::terminal_surface::TerminalSurfaceOwner::Workspace { workspace } => {
                    ("terminal", vec![workspace.as_str().into()])
                }
                crate::domain::terminal_surface::TerminalSurfaceOwner::WorkspaceTerminal {
                    workspace,
                    terminal_id,
                } => (
                    "workspace-terminal",
                    vec![workspace.as_str().into(), terminal_id.clone()],
                ),
                crate::domain::terminal_surface::TerminalSurfaceOwner::Session {
                    workspace,
                    session_id,
                } => (
                    "terminal",
                    vec![workspace.as_str().into(), session_id.clone()],
                ),
            },
            Self::RepositoryGroupState(path) => ("repository-group-state", vec![path.clone()]),
            Self::Workspaces => ("workspaces", vec![]),
            Self::NodeDetail(p, id) => ("node-detail", vec![p.clone(), id.clone()]),
            Self::SessionTab(id) => ("session-tab", vec![id.clone()]),
            Self::SessionHistory(p, count) => {
                ("session-history", vec![p.clone(), count.to_string()])
            }
            Self::Providers => ("providers", vec![]),
            Self::Branches(p, excluded) => (
                "branches",
                std::iter::once(p.clone())
                    .chain(excluded.iter().cloned())
                    .collect(),
            ),
            Self::AvailableBranches(p) => ("available-branches", vec![p.clone()]),
            Self::Issues(p) => ("issues", vec![p.clone()]),
            Self::FilteredIssues(p, filter) => {
                let mut args = vec![p.clone()];
                args.extend(filter.labels.iter().map(|label| format!("label={label}")));
                args.extend(
                    filter
                        .milestone
                        .iter()
                        .map(|milestone| format!("milestone={milestone}")),
                );
                ("issues", args)
            }
            Self::NotionTasks(request) => format_notion_tasks(request),
            Self::WorkspaceState(n, p) => ("workspace-state", vec![n.clone(), p.clone()]),
            Self::Directory(p, directory) => ("directory", vec![p.clone(), directory.clone()]),
            Self::FilteredFiles(p, filter) => {
                let mut args = vec![p.clone()];
                args.extend(
                    filter
                        .name()
                        .map(|name| format!("{FILE_FILTER_NAME_PREFIX}{}", name.as_str())),
                );
                if filter.changed() {
                    args.push(FILE_FILTER_CHANGED.into());
                }
                ("filtered-files", args)
            }
            Self::FileTab(p, file, base) => (
                "file-tab",
                vec![p.clone(), file.clone(), base.as_str().into()],
            ),
            Self::FileThreads(p, file) => ("file-threads", vec![p.clone(), file.clone()]),
            Self::ReviewThreads(name) => ("review-threads", vec![name.clone()]),
            Self::WorkflowExecution(id) => ("workflow-execution", vec![id.clone()]),
            Self::NodeArtifact(id) => ("node-artifact", vec![id.clone()]),
            Self::WorkflowOutput(id, node) => ("workflow-output", vec![id.clone(), node.clone()]),
            Self::ReviewSessionThreads(id, filter) => {
                ("review-session-threads", review_filter_parts(id, filter))
            }
            Self::ReviewWorktreeThreads(path, filter) => (
                "review-worktree-threads",
                worktree_review_filter_parts(path, filter),
            ),
            Self::ReviewSessionThread(id, thread) => {
                ("review-session-thread", vec![id.clone(), thread.clone()])
            }
            Self::ReviewSessionThreadHistory(id, thread) => (
                "review-session-thread-history",
                vec![id.clone(), thread.clone()],
            ),
            Self::Workflows => ("workflows", vec![]),
            Self::Workflow(name) => ("workflow", vec![name.clone()]),
            Self::Facets(kind) => ("facets", vec![facet_kind_name(*kind).into()]),
            Self::Diagnostics => ("diagnostics", vec![]),
            Self::DesktopSettings => ("desktop-settings", vec![]),
            Self::NotionConfig(p) => ("notion-config", vec![p.clone()]),
            Self::ProviderAvailability => ("provider-availability", vec![]),
            Self::ExternalEditor => ("external-editor", vec![]),
            Self::ReleashBase(p) => ("releash-base", vec![p.clone()]),
            Self::TerminalStartupCommand => ("terminal-startup-command", vec![]),
            Self::WorkflowConfig => ("workflow-config", vec![]),
            Self::ProviderUsage => ("provider-usage", vec![]),
            Self::ProviderHookHealth => ("provider-hook-health", vec![]),
            Self::ExecutionOverview => ("execution-overview", vec![]),
        }
    }
}

const FILE_FILTER_NAME_PREFIX: &str = "name=";
const FILE_FILTER_CHANGED: &str = "changed=true";

/// `filtered-files` の引数から絞り込みの条件を読む。
/// 引数は `name=<検索の入力>` と `changed=true` のうち指定するものを、この順で並べたものである。
fn parse_file_filter(
    args: &[&str],
) -> Result<crate::domain::code::value_objects::file_list::FileFilter, SubscriptionError> {
    use crate::domain::code::value_objects::file_list::{FileFilter, NameQuery};
    let (name, rest) = match args.split_first() {
        Some((first, rest)) if first.starts_with(FILE_FILTER_NAME_PREFIX) => (
            Some(
                NameQuery::new(&first[FILE_FILTER_NAME_PREFIX.len()..])
                    .map_err(|_| SubscriptionError::InvalidId)?,
            ),
            rest,
        ),
        _ => (None, args),
    };
    let changed = match rest {
        [] => false,
        [FILE_FILTER_CHANGED] => true,
        _ => return Err(SubscriptionError::InvalidId),
    };
    FileFilter::new(name, changed).map_err(|_| SubscriptionError::InvalidId)
}

const NOTION_TITLE_PREFIX: &str = "title=";
const NOTION_LABELS_PREFIX: &str = "labels=";

fn parse_notion_tasks(
    path: &str,
    count: &str,
    filters: &[&str],
) -> Result<SubscriptionTarget, SubscriptionError> {
    let raw_count = count;
    let count: usize = count.parse().map_err(|_| SubscriptionError::InvalidId)?;
    if count == 0 || count.to_string() != raw_count {
        return Err(SubscriptionError::InvalidId);
    }
    let mut title = None;
    let mut labels =
        std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for filter in filters {
        if let Some(value) = filter.strip_prefix(NOTION_TITLE_PREFIX) {
            if title.is_some() || value.is_empty() {
                return Err(SubscriptionError::InvalidId);
            }
            title = Some(value.into());
        } else if let Some(value) = filter.strip_prefix(NOTION_LABELS_PREFIX) {
            if !labels.is_empty() {
                return Err(SubscriptionError::InvalidId);
            }
            labels = serde_json::from_str(value).map_err(|_| SubscriptionError::InvalidId)?;
            if labels.is_empty()
                || labels.iter().any(|(key, values)| {
                    key.is_empty() || values.is_empty() || values.iter().any(String::is_empty)
                })
            {
                return Err(SubscriptionError::InvalidId);
            }
        } else {
            return Err(SubscriptionError::InvalidId);
        }
    }
    Ok(SubscriptionTarget::NotionTasks(NotionTaskListRequest {
        path: path.into(),
        count,
        title,
        labels,
    }))
}

fn format_notion_tasks(request: &NotionTaskListRequest) -> (&'static str, Vec<String>) {
    let NotionTaskListRequest {
        path,
        count,
        title,
        labels,
    } = request;
    let mut args = vec![path.into(), count.to_string()];
    if let Some(title) = title {
        args.push(format!("{NOTION_TITLE_PREFIX}{title}"));
    }
    if !labels.is_empty() {
        args.push(format!(
            "{NOTION_LABELS_PREFIX}{}",
            serde_json::to_string(labels).expect("string label filters")
        ));
    }
    ("notion-tasks", args)
}

fn facet_kind(kind: &str) -> Result<FacetKind, SubscriptionError> {
    kind.parse().map_err(|_| SubscriptionError::InvalidId)
}

fn facet_kind_name(kind: FacetKind) -> &'static str {
    match kind {
        FacetKind::Policy => "policy",
        FacetKind::Knowledge => "knowledge",
        FacetKind::Instruction => "instruction",
    }
}

impl std::fmt::Display for SubscriptionTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (name, args) = self.parts();
        write!(f, "{name}")?;
        if !args.is_empty() {
            write!(f, ":")?;
        }
        for arg in args {
            write!(f, "{}:{arg}", arg.len())?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "state_subscription_target_test.rs"]
mod state_subscription_target_tests;

fn parse_session_review_filters(
    args: &[&str],
) -> Result<crate::domain::comment::ReviewThreadFilter, SubscriptionError> {
    use crate::domain::comment::{AuthorScope, ReviewThreadFilter, State as ReviewThreadState};
    let mut filter = ReviewThreadFilter::default();
    for arg in args {
        let (key, value) = arg.split_once('=').ok_or(SubscriptionError::InvalidId)?;
        if value.is_empty() {
            return Err(SubscriptionError::InvalidId);
        }
        match key {
            "file" if filter.file.is_none() => filter.file = Some(value.into()),
            "state" if filter.state.is_none() => {
                filter.state = Some(
                    ReviewThreadState::parse(value).map_err(|_| SubscriptionError::InvalidId)?,
                )
            }
            "author" if filter.author.is_none() => {
                filter.author =
                    Some(AuthorScope::parse(value).map_err(|_| SubscriptionError::InvalidId)?)
            }
            "unread" if filter.unread.is_none() => {
                filter.unread = Some(
                    ReviewThreadFilter::parse_unread(value)
                        .map_err(|_| SubscriptionError::InvalidId)?,
                )
            }
            "thread" => filter.thread_id.push(value.into()),
            _ => return Err(SubscriptionError::InvalidId),
        }
    }
    filter.thread_id.sort();
    filter.thread_id.dedup();
    let canonical = review_filter_parts("", &filter);
    if canonical[1..]
        .iter()
        .map(String::as_str)
        .ne(args.iter().copied())
    {
        return Err(SubscriptionError::InvalidId);
    }
    Ok(filter)
}
fn parse_worktree_review_filters(
    args: &[&str],
) -> Result<crate::domain::comment::ReviewWorktreeFilter, SubscriptionError> {
    use crate::domain::comment::{ReviewWorktreeFilter, State as ReviewThreadState};
    let mut filter = ReviewWorktreeFilter::default();
    for arg in args {
        let (key, value) = arg.split_once('=').ok_or(SubscriptionError::InvalidId)?;
        if value.is_empty() {
            return Err(SubscriptionError::InvalidId);
        }
        match key {
            "file" if filter.file.is_none() => filter.file = Some(value.into()),
            "state" if filter.state.is_none() => {
                filter.state = Some(
                    ReviewThreadState::parse(value).map_err(|_| SubscriptionError::InvalidId)?,
                )
            }
            "thread" => filter.thread_id.push(value.into()),
            _ => return Err(SubscriptionError::InvalidId),
        }
    }
    filter.thread_id.sort();
    filter.thread_id.dedup();
    let canonical = worktree_review_filter_parts("", &filter);
    if canonical[1..]
        .iter()
        .map(String::as_str)
        .ne(args.iter().copied())
    {
        return Err(SubscriptionError::InvalidId);
    }
    Ok(filter)
}
fn worktree_review_filter_parts(
    id: &str,
    filter: &crate::domain::comment::ReviewWorktreeFilter,
) -> Vec<String> {
    let mut args = vec![id.into()];
    if let Some(file) = &filter.file {
        args.push(format!("file={file}"));
    }
    if let Some(state) = &filter.state {
        args.push(format!("state={}", state.as_str()));
    }
    args.extend(filter.thread_id.iter().map(|id| format!("thread={id}")));
    args
}
fn review_filter_parts(
    id: &str,
    filter: &crate::domain::comment::ReviewThreadFilter,
) -> Vec<String> {
    let mut args = vec![id.into()];
    if let Some(file) = &filter.file {
        args.push(format!("file={file}"));
    }
    if let Some(state) = &filter.state {
        args.push(format!("state={}", state.as_str()));
    }
    if let Some(author) = &filter.author {
        args.push(format!("author={}", author.as_str()));
    }
    if let Some(unread) = filter.unread {
        args.push(format!("unread={unread}"));
    }
    args.extend(filter.thread_id.iter().map(|id| format!("thread={id}")));
    args
}
