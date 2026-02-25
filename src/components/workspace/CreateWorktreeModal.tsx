import { invoke } from "@tauri-apps/api/core";
import {
	GitBranch,
	Loader2,
	NotebookPen,
	Plus,
	RefreshCw,
	StickyNote,
	TicketCheck,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { useIssues } from "@/hooks/useIssues";
import { useNotionLabelOptions } from "@/hooks/useNotionLabelOptions";
import { useNotionTasks } from "@/hooks/useNotionTasks";
import { generateIssueBranchName } from "@/lib/issueBranch";
import { trackEvent } from "@/lib/telemetry";
import { branchToDir, computeWorktreeDir } from "@/lib/worktreePath";
import type {
	BranchInfo,
	IssueInfo,
	WorktreeBranch,
	WorktreeEntry,
} from "@/types/git";
import type { NotionTask } from "@/types/notion";

function notionTaskToBranchName(title: string): string {
	const slug = title
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-|-$/g, "")
		.slice(0, 40);
	return `feat/${slug}`;
}

type CreateMode = "plain" | "branch" | "issue" | "notion";

interface CreateWorktreeModalProps {
	open: boolean;
	repoPaths: string[];
	onCreated: (rootPath: string, branchName: string, repoName: string) => void;
	onClose: () => void;
}

export function CreateWorktreeModal({
	open,
	repoPaths,
	onCreated,
	onClose,
}: CreateWorktreeModalProps) {
	const [mode, setMode] = useState<CreateMode>("plain");
	const [selectedRepoPath, setSelectedRepoPath] = useState(repoPaths[0] ?? "");
	const [branchName, setBranchName] = useState("");
	const [baseBranch, setBaseBranch] = useState("");
	const [localBranches, setLocalBranches] = useState<BranchInfo[]>([]);
	const [allBranches, setAllBranches] = useState<WorktreeBranch[]>([]);
	const [creating, setCreating] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [filter, setFilter] = useState("");

	const repoName = useMemo(
		() => selectedRepoPath.split("/").filter(Boolean).pop() ?? "",
		[selectedRepoPath],
	);

	useEffect(() => {
		if (!open) return;
		setBranchName("");
		setBaseBranch("");
		setFilter("");
		setError(null);
		setSelectedRepoPath(repoPaths[0] ?? "");
	}, [open, repoPaths]);

	useEffect(() => {
		if (!open || !selectedRepoPath) return;
		let alive = true;
		invoke<BranchInfo[]>("list_branches", { repoPath: selectedRepoPath })
			.then((result) => {
				if (!alive) return;
				setLocalBranches(result.filter((b) => !b.is_remote));
				const fallback = result.find(
					(b) => !b.is_remote && (b.name === "main" || b.name === "master"),
				);
				setBaseBranch(fallback?.name ?? "HEAD");
			})
			.catch(() => {
				if (!alive) return;
				setLocalBranches([]);
				setBaseBranch("HEAD");
			});
		invoke<WorktreeBranch[]>("list_branches_with_status", {
			repoPath: selectedRepoPath,
		})
			.then((result) => {
				if (alive) setAllBranches(result);
			})
			.catch(() => {
				if (alive) setAllBranches([]);
			});
		return () => {
			alive = false;
		};
	}, [open, selectedRepoPath]);

	const nonWorktreeBranches = useMemo(
		() => allBranches.filter((b) => b.worktree_path == null),
		[allBranches],
	);

	const filteredNonWorktreeBranches = useMemo(() => {
		if (!filter) return nonWorktreeBranches;
		const lower = filter.toLowerCase();
		return nonWorktreeBranches.filter((b) =>
			b.name.toLowerCase().includes(lower),
		);
	}, [nonWorktreeBranches, filter]);

	const handleCreate = useCallback(async () => {
		if (!branchName || !selectedRepoPath) return;
		setCreating(true);
		setError(null);
		const worktreeDir = computeWorktreeDir(selectedRepoPath);
		const dirName = branchToDir(branchName);
		const worktreePath = `${worktreeDir}/${dirName}`;
		const existingNames = allBranches.map((b) => b.name);
		const isNewBranch = !existingNames.includes(branchName);
		try {
			const entry = await invoke<WorktreeEntry>("create_worktree", {
				repoPath: selectedRepoPath,
				worktreePath,
				branch: branchName,
				createBranch: isNewBranch,
				baseBranch: isNewBranch ? baseBranch || "HEAD" : null,
			});
			trackEvent("worktree_created", {
				is_new_branch: isNewBranch ? "true" : "false",
			});
			onCreated(entry.path, entry.branch, repoName);
		} catch (e) {
			setError(String(e));
		} finally {
			setCreating(false);
		}
	}, [
		branchName,
		selectedRepoPath,
		allBranches,
		baseBranch,
		repoName,
		onCreated,
	]);

	const tabs: { mode: CreateMode; label: string; icon: React.ReactNode }[] = [
		{ mode: "plain", label: "Plain", icon: <Plus className="size-3.5" /> },
		{
			mode: "branch",
			label: "Branch",
			icon: <GitBranch className="size-3.5" />,
		},
		{
			mode: "issue",
			label: "Issue",
			icon: <TicketCheck className="size-3.5" />,
		},
		{
			mode: "notion",
			label: "Notion",
			icon: <StickyNote className="size-3.5" />,
		},
	];

	return (
		<Dialog open={open} onOpenChange={(o) => !o && onClose()}>
			<DialogContent className="max-h-[85vh] overflow-y-auto">
				<DialogHeader>
					<DialogTitle>New Worktree</DialogTitle>
					<DialogDescription>
						Create a new worktree from a branch, issue, or Notion task.
					</DialogDescription>
				</DialogHeader>

				{/* Repo selector */}
				{repoPaths.length > 1 && (
					<div className="flex items-center gap-2 text-sm">
						<span className="text-muted-foreground shrink-0">Repository:</span>
						<Select
							value={selectedRepoPath}
							onValueChange={setSelectedRepoPath}
						>
							<SelectTrigger size="sm" className="flex-1">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{repoPaths.map((rp) => (
									<SelectItem key={rp} value={rp}>
										{rp.split("/").filter(Boolean).pop()}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					</div>
				)}

				{/* Mode tabs */}
				<div className="flex border-b border-border">
					{tabs.map((tab) => (
						<button
							key={tab.mode}
							type="button"
							onClick={() => {
								setMode(tab.mode);
								setBranchName("");
								setFilter("");
								setError(null);
							}}
							className={`flex items-center gap-1.5 px-3 py-1.5 text-xs transition-colors border-b-2 ${
								mode === tab.mode
									? "border-primary text-foreground"
									: "border-transparent text-muted-foreground hover:text-foreground"
							}`}
						>
							{tab.icon}
							{tab.label}
						</button>
					))}
				</div>

				{/* Mode content */}
				<div className="min-h-[200px]">
					{mode === "plain" && (
						<PlainMode
							branchName={branchName}
							onBranchNameChange={setBranchName}
							baseBranch={baseBranch}
							onBaseBranchChange={setBaseBranch}
							localBranches={localBranches}
						/>
					)}
					{mode === "branch" && (
						<BranchMode
							branches={filteredNonWorktreeBranches}
							filter={filter}
							onFilterChange={setFilter}
							onSelect={(name) => setBranchName(name)}
							selectedBranch={branchName}
						/>
					)}
					{mode === "issue" && selectedRepoPath && (
						<IssueMode
							repoPath={selectedRepoPath}
							onSelect={(issue) =>
								setBranchName(generateIssueBranchName(issue.number))
							}
							selectedBranch={branchName}
						/>
					)}
					{mode === "notion" && selectedRepoPath && (
						<NotionMode
							repoPath={selectedRepoPath}
							onSelect={(task) => {
								setBranchName(
									task.branch_name || notionTaskToBranchName(task.title),
								);
							}}
							selectedBranch={branchName}
						/>
					)}
				</div>

				{error && <p className="text-sm text-destructive">{error}</p>}

				{branchName && (
					<div className="text-xs text-muted-foreground">
						Branch: <code className="font-mono">{branchName}</code>
					</div>
				)}

				<DialogFooter>
					<Button variant="outline" onClick={onClose} disabled={creating}>
						Cancel
					</Button>
					<Button onClick={handleCreate} disabled={!branchName || creating}>
						{creating && <Loader2 className="size-3.5 mr-1 animate-spin" />}
						{creating ? "Creating..." : "Create"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function PlainMode({
	branchName,
	onBranchNameChange,
	baseBranch,
	onBaseBranchChange,
	localBranches,
}: {
	branchName: string;
	onBranchNameChange: (name: string) => void;
	baseBranch: string;
	onBaseBranchChange: (name: string) => void;
	localBranches: BranchInfo[];
}) {
	return (
		<div className="space-y-3">
			<div>
				<span className="text-xs text-muted-foreground">Branch name</span>
				<Input
					value={branchName}
					onChange={(e) => onBranchNameChange(e.target.value)}
					placeholder="feat/my-feature"
					autoFocus
				/>
			</div>
			<div>
				<span className="text-xs text-muted-foreground">Base branch</span>
				<Select value={baseBranch} onValueChange={onBaseBranchChange}>
					<SelectTrigger size="sm" className="w-full">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{localBranches.map((b) => (
							<SelectItem key={b.name} value={b.name}>
								{b.name}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>
		</div>
	);
}

function BranchMode({
	branches,
	filter,
	onFilterChange,
	onSelect,
	selectedBranch,
}: {
	branches: WorktreeBranch[];
	filter: string;
	onFilterChange: (filter: string) => void;
	onSelect: (name: string) => void;
	selectedBranch: string;
}) {
	return (
		<div className="space-y-2">
			<Input
				value={filter}
				onChange={(e) => onFilterChange(e.target.value)}
				placeholder="Filter branches..."
				autoFocus
			/>
			<ScrollArea className="h-[180px]">
				<div className="space-y-0.5">
					{branches.map((b) => (
						<button
							key={b.name}
							type="button"
							onClick={() => onSelect(b.name)}
							className={`flex w-full items-center gap-2 px-2 py-1.5 text-sm rounded transition-colors ${
								selectedBranch === b.name
									? "bg-muted text-foreground"
									: "hover:bg-secondary"
							}`}
						>
							<GitBranch className="size-3.5 text-muted-foreground shrink-0" />
							<span className="truncate">{b.name}</span>
						</button>
					))}
					{branches.length === 0 && (
						<div className="text-xs text-muted-foreground text-center py-4">
							No branches without worktrees
						</div>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}

function IssueMode({
	repoPath,
	onSelect,
	selectedBranch,
}: {
	repoPath: string;
	onSelect: (issue: IssueInfo) => void;
	selectedBranch: string;
}) {
	const { issues, loading, refresh } = useIssues(repoPath);
	const [filter, setFilter] = useState("");
	const [labelFilter, setLabelFilter] = useState("");
	const [milestoneFilter, setMilestoneFilter] = useState("");

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
		return { titles: Array.from(set).sort(), hasNone };
	}, [issues]);

	const filtered = useMemo(() => {
		let result = [...issues];
		if (filter) {
			const lower = filter.toLowerCase();
			result = result.filter(
				(i) =>
					i.title.toLowerCase().includes(lower) ||
					String(i.number).includes(lower),
			);
		}
		if (labelFilter) {
			result = result.filter((i) =>
				i.labels.some((l) => l.name === labelFilter),
			);
		}
		if (milestoneFilter) {
			if (milestoneFilter === "__none__") {
				result = result.filter((i) => i.milestone === null);
			} else {
				result = result.filter((i) => i.milestone?.title === milestoneFilter);
			}
		}
		result.sort((a, b) => b.number - a.number);
		return result;
	}, [issues, filter, labelFilter, milestoneFilter]);

	return (
		<div className="space-y-2">
			<div className="flex gap-1">
				<Input
					value={filter}
					onChange={(e) => setFilter(e.target.value)}
					placeholder="Filter issues..."
					autoFocus
					className="flex-1"
				/>
				<Button
					size="icon"
					variant="ghost"
					className="size-8 shrink-0"
					onClick={refresh}
					aria-label="Issue一覧を再取得"
					title="Issue一覧を再取得"
				>
					<RefreshCw className="size-3.5" />
				</Button>
			</div>
			{allLabels.length > 0 && (
				<Select
					value={labelFilter}
					onValueChange={(v) => setLabelFilter(v === "__all__" ? "" : v)}
				>
					<SelectTrigger size="sm" className="w-full">
						<SelectValue placeholder="All labels" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="__all__">All labels</SelectItem>
						{allLabels.map((label) => (
							<SelectItem key={label} value={label}>
								{label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			)}
			{(allMilestones.titles.length > 0 || allMilestones.hasNone) && (
				<Select
					value={milestoneFilter}
					onValueChange={(v) => setMilestoneFilter(v === "__all__" ? "" : v)}
				>
					<SelectTrigger size="sm" className="w-full">
						<SelectValue placeholder="All milestones" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="__all__">All milestones</SelectItem>
						{allMilestones.hasNone && (
							<SelectItem value="__none__">No milestone</SelectItem>
						)}
						{allMilestones.titles.map((title) => (
							<SelectItem key={title} value={title}>
								{title}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			)}
			<ScrollArea className="h-[180px]">
				{loading ? (
					<div className="flex items-center justify-center py-8">
						<Loader2 className="size-4 text-muted-foreground animate-spin" />
					</div>
				) : (
					<div className="space-y-0.5">
						{filtered.map((issue) => {
							const isSelected =
								selectedBranch === generateIssueBranchName(issue.number);
							return (
								<button
									key={issue.number}
									type="button"
									onClick={() => onSelect(issue)}
									className={`flex w-full items-start gap-2 px-2 py-1.5 text-sm rounded transition-colors text-left ${
										isSelected
											? "bg-muted text-foreground"
											: "hover:bg-secondary"
									}`}
								>
									<TicketCheck className="size-3.5 text-muted-foreground shrink-0 mt-0.5" />
									<div className="min-w-0 flex-1">
										<div>
											<span className="text-muted-foreground">
												#{issue.number}
											</span>{" "}
											<span>{issue.title}</span>
										</div>
										{issue.labels.length > 0 && (
											<div className="flex flex-wrap gap-1 mt-0.5">
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
									</div>
								</button>
							);
						})}
						{filtered.length === 0 && (
							<div className="text-xs text-muted-foreground text-center py-4">
								No issues found
							</div>
						)}
					</div>
				)}
			</ScrollArea>
		</div>
	);
}

function NotionMode({
	repoPath,
	onSelect,
	selectedBranch,
}: {
	repoPath: string;
	onSelect: (task: NotionTask) => void;
	selectedBranch: string;
}) {
	const { tasks, loading, loadMore, hasMore, search } =
		useNotionTasks(repoPath);
	const { labelOptions } = useNotionLabelOptions(repoPath);
	const [titleFilter, setTitleFilter] = useState("");
	const [labelFilters, setLabelFilters] = useState<Record<string, string>>({});

	const handleTitleChange = useCallback(
		(value: string) => {
			setTitleFilter(value);
			search(value, labelFilters);
		},
		[labelFilters, search],
	);

	const handleLabelChange = useCallback(
		(propertyName: string, value: string) => {
			const newFilters = { ...labelFilters, [propertyName]: value };
			setLabelFilters(newFilters);
			search(titleFilter, newFilters);
		},
		[titleFilter, labelFilters, search],
	);

	return (
		<div className="space-y-2">
			<Input
				value={titleFilter}
				onChange={(e) => handleTitleChange(e.target.value)}
				placeholder="Filter Notion tasks..."
				autoFocus
			/>
			{labelOptions.map((opt) => (
				<Select
					key={opt.property_name}
					value={labelFilters[opt.property_name] ?? ""}
					onValueChange={(v) =>
						handleLabelChange(opt.property_name, v === "__all__" ? "" : v)
					}
				>
					<SelectTrigger size="sm" className="w-full">
						<SelectValue placeholder={`${opt.property_name}: All`} />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="__all__">{opt.property_name}: All</SelectItem>
						{opt.options.map((v, i) => (
							<SelectItem key={v} value={opt.option_ids[i] ?? v}>
								{v}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			))}
			<ScrollArea className="h-[180px]">
				{loading ? (
					<div className="flex items-center justify-center py-8">
						<Loader2 className="size-4 text-muted-foreground animate-spin" />
					</div>
				) : (
					<div className="space-y-0.5">
						{tasks.map((task) => {
							const isSelected =
								selectedBranch ===
								(task.branch_name || notionTaskToBranchName(task.title));
							return (
								<button
									key={task.id}
									type="button"
									onClick={() => onSelect(task)}
									className={`flex w-full items-start gap-2 px-2 py-1.5 text-sm rounded transition-colors text-left ${
										isSelected
											? "bg-muted text-foreground"
											: "hover:bg-secondary"
									}`}
								>
									<NotebookPen className="size-3.5 text-muted-foreground shrink-0 mt-0.5" />
									<div className="min-w-0 flex-1">
										<span>{task.title}</span>
										{Object.keys(task.labels).length > 0 && (
											<div className="flex flex-wrap gap-1 mt-0.5">
												{Object.entries(task.labels).flatMap(([prop, values]) =>
													values.map((label) => (
														<span
															key={`${prop}:${label}`}
															className="inline-flex items-center rounded-full px-1.5 py-0.5 text-[9px] font-medium leading-none bg-muted text-muted-foreground border border-border"
														>
															{label}
														</span>
													)),
												)}
											</div>
										)}
									</div>
								</button>
							);
						})}
						{tasks.length === 0 && (
							<div className="text-xs text-muted-foreground text-center py-4">
								No tasks found
							</div>
						)}
						{hasMore && (
							<Button
								size="sm"
								variant="ghost"
								className="w-full text-xs"
								onClick={loadMore}
							>
								Load more
							</Button>
						)}
					</div>
				)}
			</ScrollArea>
		</div>
	);
}
