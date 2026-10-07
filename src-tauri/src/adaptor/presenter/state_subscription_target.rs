use crate::domain::code::{ReviewBase, ReviewSection};
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
        let target = match (name, args) {
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
            ("repository-paths", []) => Ok(Self::RepositoryPaths),
            ("workspaces", []) => Ok(Self::Workspaces),
            ("selection", [path, id]) => Ok(Self::Selection((*path).into(), (*id).into())),
            ("node-detail", [path, id]) => Ok(Self::NodeDetail((*path).into(), (*id).into())),
            ("agent-session", [id]) => Ok(Self::AgentSession((*id).into())),
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
            ("branch-base", [path, name]) => Ok(Self::BranchBase((*path).into(), (*name).into())),
            ("branch-status", [path]) => Ok(Self::BranchStatus((*path).into())),
            ("current-branch", [path]) => Ok(Self::CurrentBranch((*path).into())),
            ("issues", [path]) => Ok(Self::Issues((*path).into())),
            ("notion-label-options", [path]) => Ok(Self::NotionLabelOptions((*path).into())),
            ("notion-tasks", [path, count, filters @ ..]) => {
                parse_notion_tasks(path, count, filters)
            }
            ("worktrees", [path]) => Ok(Self::Worktrees((*path).into())),
            ("startup-repository", []) => Ok(Self::StartupRepository),
            ("workspace-state", [name, path]) => {
                Ok(Self::WorkspaceState((*name).into(), (*path).into()))
            }
            ("review-snapshot", [path, base]) => Ok(Self::ReviewSnapshot(
                (*path).into(),
                ReviewBase::parse(base).map_err(|_| SubscriptionError::InvalidId)?,
            )),
            ("review-file-view", [path, file, section, base]) => Ok(Self::ReviewFileView(
                (*path).into(),
                (*file).into(),
                ReviewSection::parse(section).map_err(|_| SubscriptionError::InvalidId)?,
                ReviewBase::parse(base).map_err(|_| SubscriptionError::InvalidId)?,
            )),
            ("review-threads", [name]) => Ok(Self::ReviewThreads((*name).into())),
            ("workflow-execution", [id]) => Ok(Self::WorkflowExecution((*id).into())),
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
            ("workflow-source", [name]) => Ok(Self::WorkflowSource((*name).into())),
            ("facets", [kind]) => Ok(Self::Facets(facet_kind(kind)?)),
            ("facet", [kind, key]) => Ok(Self::Facet(facet_kind(kind)?, (*key).into())),
            ("diagnostics", []) => Ok(Self::Diagnostics),
            ("desktop-settings", []) => Ok(Self::DesktopSettings),
            ("notion-config", [path]) => Ok(Self::NotionConfig((*path).into())),
            ("provider-availability", []) => Ok(Self::ProviderAvailability),
            ("external-editor", []) => Ok(Self::ExternalEditor),
            ("releash-base", [path]) => Ok(Self::ReleashBase((*path).into())),
            ("workflow-config", []) => Ok(Self::WorkflowConfig),
            ("provider-hook-health", []) => Ok(Self::ProviderHookHealth),
            _ => Err(SubscriptionError::UnknownTarget),
        }?;
        Ok(target)
    }
}

impl SubscriptionTarget {
    pub fn parts(&self) -> (&'static str, Vec<String>) {
        match self {
            Self::Terminal(owner) => (
                "terminal",
                match owner {
                    crate::domain::terminal_surface::TerminalSurfaceOwner::Workspace {
                        workspace,
                    } => vec![workspace.as_str().into()],
                    crate::domain::terminal_surface::TerminalSurfaceOwner::Session {
                        workspace,
                        session_id,
                    } => vec![workspace.as_str().into(), session_id.clone()],
                },
            ),
            Self::RepositoryPaths => ("repository-paths", vec![]),
            Self::Workspaces => ("workspaces", vec![]),
            Self::Selection(p, id) => ("selection", vec![p.clone(), id.clone()]),
            Self::NodeDetail(p, id) => ("node-detail", vec![p.clone(), id.clone()]),
            Self::AgentSession(id) => ("agent-session", vec![id.clone()]),
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
            Self::BranchBase(p, n) => ("branch-base", vec![p.clone(), n.clone()]),
            Self::BranchStatus(p) => ("branch-status", vec![p.clone()]),
            Self::CurrentBranch(p) => ("current-branch", vec![p.clone()]),
            Self::Issues(p) => ("issues", vec![p.clone()]),
            Self::NotionLabelOptions(p) => ("notion-label-options", vec![p.clone()]),
            Self::NotionTasks(request) => format_notion_tasks(request),
            Self::Worktrees(p) => ("worktrees", vec![p.clone()]),
            Self::StartupRepository => ("startup-repository", vec![]),
            Self::WorkspaceState(n, p) => ("workspace-state", vec![n.clone(), p.clone()]),
            Self::ReviewSnapshot(p, base) => {
                ("review-snapshot", vec![p.clone(), base.as_str().into()])
            }
            Self::ReviewFileView(p, file, section, base) => (
                "review-file-view",
                vec![
                    p.clone(),
                    file.clone(),
                    section.as_str().into(),
                    base.as_str().into(),
                ],
            ),
            Self::ReviewThreads(name) => ("review-threads", vec![name.clone()]),
            Self::WorkflowExecution(id) => ("workflow-execution", vec![id.clone()]),
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
            Self::WorkflowSource(name) => ("workflow-source", vec![name.clone()]),
            Self::Facets(kind) => ("facets", vec![facet_kind_name(*kind).into()]),
            Self::Facet(kind, key) => ("facet", vec![facet_kind_name(*kind).into(), key.clone()]),
            Self::Diagnostics => ("diagnostics", vec![]),
            Self::DesktopSettings => ("desktop-settings", vec![]),
            Self::NotionConfig(p) => ("notion-config", vec![p.clone()]),
            Self::ProviderAvailability => ("provider-availability", vec![]),
            Self::ExternalEditor => ("external-editor", vec![]),
            Self::ReleashBase(p) => ("releash-base", vec![p.clone()]),
            Self::WorkflowConfig => ("workflow-config", vec![]),
            Self::ProviderHookHealth => ("provider-hook-health", vec![]),
        }
    }
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
    match kind {
        "policy" => Ok(FacetKind::Policy),
        "knowledge" => Ok(FacetKind::Knowledge),
        "instruction" => Ok(FacetKind::Instruction),
        _ => Err(SubscriptionError::InvalidId),
    }
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
    use crate::domain::comment::{AuthorScope, ReviewThreadFilter, ReviewThreadState};
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
    use crate::domain::comment::{ReviewThreadState, ReviewWorktreeFilter};
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
