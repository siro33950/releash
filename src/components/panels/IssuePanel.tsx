import { invoke } from "@tauri-apps/api/core";
import {
	ChevronDown,
	ChevronRight,
	ExternalLink,
	GitBranch,
	Loader2,
	RefreshCw,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
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
					<div className="px-3 py-4 text-xs text-muted-foreground text-center">
						No repositories
					</div>
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
	const [expanded, setExpanded] = useState(defaultExpanded);
	const [titleFilter, setTitleFilter] = useState("");
	const [labelFilter, setLabelFilter] = useState("");
	const { issues, loading, refresh } = useIssues(repoPath);
	const repoName = repoPath.split("/").filter(Boolean).pop() ?? "repo";

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

		result.sort((a, b) => b.number - a.number);

		return result;
	}, [issues, titleFilter, labelFilter]);

	return (
		<div className="border-b border-border">
			<button
				type="button"
				className="flex items-center gap-1.5 w-full px-3 py-1.5 text-xs font-medium hover:bg-accent/50"
				onClick={() => setExpanded((v) => !v)}
			>
				{expanded ? (
					<ChevronDown className="size-3 shrink-0" />
				) : (
					<ChevronRight className="size-3 shrink-0" />
				)}
				<span className="truncate">{repoName}</span>
				{loading && <Loader2 className="size-3 animate-spin ml-auto" />}
				{!loading && (
					<span className="ml-auto text-muted-foreground">{issues.length}</span>
				)}
			</button>
			{expanded && (
				<div className="pb-1">
					{!isAvailable && (
						<div className="px-3 py-2 text-[10px] text-muted-foreground">
							GitHub CLI (gh) が利用できません
						</div>
					)}
					{isAvailable && !loading && issues.length > 0 && (
						<div className="px-2 py-1.5 flex flex-col gap-1">
							<input
								type="text"
								placeholder="Filter by title..."
								value={titleFilter}
								onChange={(e) => setTitleFilter(e.target.value)}
								className="w-full bg-muted border border-border rounded px-2 py-1 text-[10px] focus:outline-none focus:ring-1 focus:ring-primary"
							/>
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
						</div>
					)}
					{isAvailable && !loading && issues.length === 0 && (
						<div className="px-3 py-2 text-[10px] text-muted-foreground">
							No open issues
						</div>
					)}
					{isAvailable &&
						!loading &&
						issues.length > 0 &&
						filteredIssues.length === 0 && (
							<div className="px-3 py-2 text-[10px] text-muted-foreground">
								No matching issues
							</div>
						)}
					{isAvailable &&
						filteredIssues.map((issue) => (
							<IssueCard
								key={issue.number}
								issue={issue}
								repoPath={repoPath}
								repoName={repoName}
								onSelectWorktree={onSelectWorktree}
							/>
						))}
					{isAvailable && !loading && (
						<div className="px-3 pt-1">
							<Button
								variant="ghost"
								size="sm"
								className="h-5 px-1.5 text-[10px] text-muted-foreground"
								onClick={refresh}
							>
								<RefreshCw className="size-2.5 mr-1" />
								Refresh
							</Button>
						</div>
					)}
				</div>
			)}
		</div>
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
}

function IssueCard({
	issue,
	repoPath,
	repoName,
	onSelectWorktree,
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
			onSelectWorktree(entry.path, branchName, repoName);
		} catch (e) {
			setError(String(e));
		} finally {
			setCreating(false);
		}
	}, [issue.number, repoPath, repoName, onSelectWorktree]);

	const createdDate = new Date(issue.created_at).toLocaleDateString();

	return (
		<div className="mx-2 mb-1 rounded border border-border bg-card p-2">
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

			{issue.labels.length > 0 && (
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
				</div>
			)}

			<div className="flex items-center gap-2 mt-1.5 text-[10px] text-muted-foreground">
				{issue.assignees.length > 0 && (
					<span>{issue.assignees.map((a) => a.login).join(", ")}</span>
				)}
				<span className="ml-auto">{createdDate}</span>
			</div>

			{error && (
				<div className="mt-1.5 text-[10px] text-destructive break-all">
					{error}
				</div>
			)}

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
		</div>
	);
}
