import { invoke } from "@tauri-apps/api/core";
import {
	ExternalLink,
	FolderOpen,
	GitBranch,
	Loader2,
	RefreshCw,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { EmptyState } from "@/components/panels/EmptyState";
import { Button } from "@/components/ui/button";
import { CollapsibleSection } from "@/components/ui/collapsible-section";
import { Input } from "@/components/ui/input";
import { Message } from "@/components/ui/message";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useIssues } from "@/hooks/useIssues";
import { generateIssueBranchName } from "@/lib/issueBranch";
import { branchToDir, computeWorktreeDir } from "@/lib/worktreePath";
import type { IssueInfo, ProviderStatus, WorktreeEntry } from "@/types/git";

interface IssuePanelProps {
	repoPaths: string[];
	providerStatuses: Record<string, ProviderStatus | null>;
	onSelectWorktree: (
		path: string,
		branchName?: string,
		repoName?: string,
	) => void;
}

export function IssuePanel({
	repoPaths,
	providerStatuses,
	onSelectWorktree,
}: IssuePanelProps) {
	return (
		<div className="h-full flex flex-col bg-sidebar">
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate">
					Issues
				</span>
			</div>
			<ScrollArea className="flex-1 min-h-0">
				{repoPaths.map((repoPath) => (
					<RepoIssueSection
						key={repoPath}
						repoPath={repoPath}
						providerStatus={providerStatuses[repoPath] ?? null}
						onSelectWorktree={onSelectWorktree}
						defaultExpanded={repoPaths.length === 1}
					/>
				))}
				{repoPaths.length === 0 && (
					<EmptyState
						compact
						title="No repositories"
						className="px-3 py-4 text-center"
					/>
				)}
			</ScrollArea>
		</div>
	);
}

interface RepoIssueSectionProps {
	repoPath: string;
	providerStatus: ProviderStatus | null;
	onSelectWorktree: (
		path: string,
		branchName?: string,
		repoName?: string,
	) => void;
	defaultExpanded: boolean;
}

function RepoIssueSection({
	repoPath,
	providerStatus,
	onSelectWorktree,
	defaultExpanded,
}: RepoIssueSectionProps) {
	const [titleFilter, setTitleFilter] = useState("");
	const [labelFilter, setLabelFilter] = useState("");
	const [milestoneFilter, setMilestoneFilter] = useState("");
	const { issues, loading, refresh } = useIssues(repoPath);
	const [worktrees, setWorktrees] = useState<WorktreeEntry[]>([]);
	const repoName = repoPath.split("/").filter(Boolean).pop() ?? "repo";

	const fetchWorktrees = useCallback(async () => {
		try {
			const result = await invoke<WorktreeEntry[]>("list_worktrees", {
				repoPath,
			});
			setWorktrees(result);
		} catch {
			// noop: preserve previous worktrees on error
		}
	}, [repoPath]);

	useEffect(() => {
		fetchWorktrees();
	}, [fetchWorktrees]);

	const isAvailable = providerStatus === "available" || providerStatus === null;

	const allLabels = useMemo(() => {
		const set = new Set<string>();
		for (const issue of issues) {
			for (const label of issue.labels) {
				set.add(label.name);
			}
		}
		return Array.from(set).sort();
	}, [issues]);

	const allMilestones = useMemo(() => {
		const set = new Set<string>();
		let hasNone = false;
		for (const issue of issues) {
			if (issue.milestone) {
				set.add(issue.milestone.title);
			} else {
				hasNone = true;
			}
		}
		const sorted = Array.from(set).sort();
		return { titles: sorted, hasNone };
	}, [issues]);

	const hasActiveFilters =
		titleFilter !== "" || labelFilter !== "" || milestoneFilter !== "";

	const clearFilters = useCallback(() => {
		setTitleFilter("");
		setLabelFilter("");
		setMilestoneFilter("");
	}, []);

	const filteredIssues = useMemo(() => {
		let result = [...issues];

		if (titleFilter) {
			const lower = titleFilter.toLowerCase();
			result = result.filter((issue) =>
				issue.title.toLowerCase().includes(lower),
			);
		}

		if (labelFilter) {
			result = result.filter((issue) =>
				issue.labels.some((l) => l.name === labelFilter),
			);
		}

		if (milestoneFilter) {
			if (milestoneFilter === "__none__") {
				result = result.filter((issue) => issue.milestone === null);
			} else {
				result = result.filter(
					(issue) => issue.milestone?.title === milestoneFilter,
				);
			}
		}

		result.sort((a, b) => b.number - a.number);

		return result;
	}, [issues, titleFilter, labelFilter, milestoneFilter]);

	return (
		<CollapsibleSection
			title={repoName}
			defaultOpen={defaultExpanded}
			className="border-b border-border"
			actions={
				<>
					{loading && <Loader2 className="size-3 animate-spin ml-auto" />}
					{!loading && (
						<span className="ml-auto text-muted-foreground">
							{issues.length}
						</span>
					)}
					<Button
						variant="ghost"
						size="icon"
						aria-label="Refresh"
						title="Refresh"
						className="h-5 w-5 min-w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-accent-foreground/10 transition-colors shrink-0"
						onClick={(e) => {
							e.stopPropagation();
							refresh();
						}}
					>
						<RefreshCw className="size-3" />
					</Button>
				</>
			}
		>
			<div className="pb-1">
				{!isAvailable && (
					<div className="px-3 py-2 text-[10px] text-muted-foreground">
						GitHub CLI (gh) が利用できません
					</div>
				)}
				{isAvailable && !loading && issues.length > 0 && (
					<div className="px-2 py-1.5 flex flex-col gap-1">
						<div className="flex items-center gap-1">
							<Input
								type="text"
								variant="panel"
								size="xs"
								placeholder="Filter by title..."
								aria-label="Filter issues by title"
								value={titleFilter}
								onChange={(e) => setTitleFilter(e.target.value)}
							/>
							{hasActiveFilters && (
								<button
									type="button"
									onClick={clearFilters}
									className="shrink-0 p-0.5 text-muted-foreground hover:text-foreground rounded"
									aria-label="Clear filters"
								>
									<X className="size-3" />
								</button>
							)}
						</div>
						{allLabels.length > 0 && (
							<select
								value={labelFilter}
								onChange={(e) => setLabelFilter(e.target.value)}
								className="w-full bg-muted border border-border rounded px-2 py-1 text-[10px] focus:outline-none focus:ring-1 focus:ring-primary"
							>
								<option value="">All labels</option>
								{allLabels.map((label) => (
									<option key={label} value={label}>
										{label}
									</option>
								))}
							</select>
						)}
						{allMilestones.titles.length > 0 && (
							<select
								value={milestoneFilter}
								onChange={(e) => setMilestoneFilter(e.target.value)}
								className="w-full bg-muted border border-border rounded px-2 py-1 text-[10px] focus:outline-none focus:ring-1 focus:ring-primary"
							>
								<option value="">All milestones</option>
								{allMilestones.hasNone && (
									<option value="__none__">未設定</option>
								)}
								{allMilestones.titles.map((title) => (
									<option key={title} value={title}>
										{title}
									</option>
								))}
							</select>
						)}
					</div>
				)}
				{isAvailable && !loading && issues.length === 0 && (
					<EmptyState
						compact
						title="No open issues"
						className="px-3 py-2 text-[10px]"
					/>
				)}
				{isAvailable &&
					!loading &&
					issues.length > 0 &&
					filteredIssues.length === 0 && (
						<EmptyState
							compact
							title="No matching issues"
							className="px-3 py-2 text-[10px]"
						>
							<button
								type="button"
								className="ml-2 text-primary hover:underline"
								onClick={clearFilters}
							>
								Clear filters
							</button>
						</EmptyState>
					)}
				{isAvailable &&
					filteredIssues.map((issue) => {
						const branchName = generateIssueBranchName(issue.number);
						const matchedWorktree = worktrees.find(
							(wt) => wt.branch === branchName,
						);
						return (
							<IssueCard
								key={issue.number}
								issue={issue}
								repoPath={repoPath}
								repoName={repoName}
								onSelectWorktree={onSelectWorktree}
								existingWorktree={matchedWorktree}
								onWorktreeCreated={fetchWorktrees}
							/>
						);
					})}
			</div>
		</CollapsibleSection>
	);
}

interface IssueCardProps {
	issue: IssueInfo;
	repoPath: string;
	repoName: string;
	onSelectWorktree: (
		path: string,
		branchName?: string,
		repoName?: string,
	) => void;
	existingWorktree?: WorktreeEntry;
	onWorktreeCreated: () => void;
}

function IssueCard({
	issue,
	repoPath,
	repoName,
	onSelectWorktree,
	existingWorktree,
	onWorktreeCreated,
}: IssueCardProps) {
	const [creating, setCreating] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const handleCreateWorktree = useCallback(async () => {
		setCreating(true);
		setError(null);
		try {
			const branchName = generateIssueBranchName(issue.number);
			const worktreeDir = computeWorktreeDir(repoPath);
			const worktreePath = `${worktreeDir}/${branchToDir(branchName)}`;

			let defaultBranch: string;
			try {
				defaultBranch = await invoke<string>("get_default_branch", {
					repoPath,
				});
			} catch {
				defaultBranch = "main";
			}

			const entry = await invoke<WorktreeEntry>("create_worktree", {
				repoPath,
				worktreePath,
				branch: branchName,
				createBranch: true,
				baseBranch: defaultBranch,
			});
			onWorktreeCreated();
			onSelectWorktree(entry.path, branchName, repoName);
		} catch (e) {
			setError(String(e));
		} finally {
			setCreating(false);
		}
	}, [issue.number, repoPath, repoName, onSelectWorktree, onWorktreeCreated]);

	const handleOpenWorktree = useCallback(() => {
		const branchName = generateIssueBranchName(issue.number);
		if (existingWorktree) {
			onSelectWorktree(existingWorktree.path, branchName, repoName);
		}
	}, [issue.number, existingWorktree, onSelectWorktree, repoName]);

	const createdDate = new Date(issue.created_at).toLocaleDateString();

	return (
		<div className="mx-2 mb-1 rounded border border-border bg-card p-2 shadow-sm transition-[border-color,box-shadow] hover:shadow-md hover:border-primary/30">
			<div className="flex items-start gap-1.5">
				<span className="text-[10px] text-muted-foreground shrink-0">
					#{issue.number}
				</span>
				<span className="text-xs font-medium leading-tight flex-1 break-words">
					{issue.title}
				</span>
				<a
					href={issue.url}
					target="_blank"
					rel="noopener noreferrer"
					className="shrink-0 text-muted-foreground hover:text-foreground"
				>
					<ExternalLink className="size-3" />
				</a>
			</div>

			{(issue.labels.length > 0 || issue.milestone) && (
				<div className="flex flex-wrap gap-1 mt-1.5">
					{issue.labels.map((label) => {
						const hasColor = /^[0-9a-fA-F]{6}$/.test(label.color);
						return (
							<span
								key={label.name}
								className="inline-flex items-center rounded-full px-1.5 py-0.5 text-[9px] font-medium leading-none"
								style={
									hasColor
										? {
												backgroundColor: `#${label.color}20`,
												color: `#${label.color}`,
												border: `1px solid #${label.color}40`,
											}
										: {
												backgroundColor: "var(--color-muted)",
												color: "var(--color-muted-foreground)",
												border: "1px solid var(--color-border)",
											}
								}
							>
								{label.name}
							</span>
						);
					})}
					{issue.milestone && (
						<span className="inline-flex items-center rounded-full px-1.5 py-0.5 text-[9px] font-medium leading-none bg-muted text-muted-foreground border border-border">
							{issue.milestone.title}
						</span>
					)}
				</div>
			)}

			<div className="flex items-center gap-2 mt-1.5 text-[10px] text-muted-foreground">
				{issue.assignees.length > 0 && (
					<span>{issue.assignees.map((a) => a.login).join(", ")}</span>
				)}
				<span className="ml-auto">{createdDate}</span>
			</div>

			{error && <Message message={error} size="xs" className="mt-1.5" />}

			{existingWorktree ? (
				<Button
					variant="outline"
					size="sm"
					className="w-full mt-2 h-6 text-[10px]"
					onClick={handleOpenWorktree}
				>
					<FolderOpen className="size-3 mr-1" />
					Open Worktree
				</Button>
			) : (
				<Button
					variant="outline"
					size="sm"
					className="w-full mt-2 h-6 text-[10px]"
					onClick={handleCreateWorktree}
					disabled={creating}
				>
					{creating ? (
						<Loader2 className="size-3 mr-1 animate-spin" />
					) : (
						<GitBranch className="size-3 mr-1" />
					)}
					Create Worktree
				</Button>
			)}
		</div>
	);
}
