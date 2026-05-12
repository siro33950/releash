import { invoke } from "@tauri-apps/api/core";
import {
	Check,
	GitBranch,
	Loader2,
	Plus,
	RefreshCw,
	StickyNote,
	TicketCheck,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
	Command,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
} from "@/components/ui/command";
import {
	Dialog,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useIssues } from "@/hooks/useIssues";
import { useNotionLabelOptions } from "@/hooks/useNotionLabelOptions";
import { useNotionTasks } from "@/hooks/useNotionTasks";
import { generateIssueBranchName } from "@/lib/issueBranch";
import { trackEvent } from "@/lib/telemetry";
import { cn } from "@/lib/utils";
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
	const [selectedBranches, setSelectedBranches] = useState<string[]>([]);
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

	const toggleBranch = useCallback((branch: string) => {
		setSelectedBranches((prev) =>
			prev.includes(branch)
				? prev.filter((b) => b !== branch)
				: [...prev, branch],
		);
	}, []);

	useEffect(() => {
		if (!open) return;
		setMode("plain");
		setSelectedBranches([]);
		setBaseBranch("");
		setFilter("");
		setError(null);
		setSelectedRepoPath(repoPaths[0] ?? "");
	}, [open, repoPaths]);

	useEffect(() => {
		if (!open || !selectedRepoPath) return;
		let alive = true;
		setLocalBranches([]);
		setAllBranches([]);
		setBaseBranch("HEAD");
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

	const worktreeBranchNames = useMemo(
		() =>
			new Set(
				allBranches.filter((b) => b.worktree_path != null).map((b) => b.name),
			),
		[allBranches],
	);

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
		if (selectedBranches.length === 0 || !selectedRepoPath) return;
		setCreating(true);
		setError(null);
		const worktreeDir = computeWorktreeDir(selectedRepoPath);
		const existingNames = allBranches.map((b) => b.name);

		try {
			const createdEntries: WorktreeEntry[] = [];
			const failedBranches: string[] = [];

			for (const branch of selectedBranches) {
				const dirName = branchToDir(branch);
				const worktreePath = `${worktreeDir}/${dirName}`;
				const isNewBranch = !existingNames.includes(branch);
				try {
					const entry = await invoke<WorktreeEntry>("create_worktree", {
						repoPath: selectedRepoPath,
						worktreePath,
						branch,
						createBranch: isNewBranch,
						baseBranch: isNewBranch ? baseBranch || "HEAD" : null,
					});
					createdEntries.push(entry);
				} catch {
					failedBranches.push(branch);
				}
			}

			if (createdEntries.length > 0) {
				const lastEntry = createdEntries[createdEntries.length - 1];
				trackEvent("worktree_created", {
					count: String(createdEntries.length),
				});
				onCreated(lastEntry.path, lastEntry.branch, repoName);
			}

			if (failedBranches.length > 0) {
				setError(`Failed to create: ${failedBranches.join(", ")}`);
			}
		} finally {
			setCreating(false);
		}
	}, [
		selectedBranches,
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
			<DialogContent
				showCloseButton={false}
				className="h-[70vh] sm:max-w-2xl flex flex-col overflow-hidden"
			>
				{/* Header — title + repo selector side by side */}
				<DialogHeader className="flex-row items-center justify-between gap-4">
					<DialogTitle>New Worktree</DialogTitle>
					{repoPaths.length > 1 && (
						<Select
							value={selectedRepoPath}
							onValueChange={(nextRepoPath) => {
								setSelectedRepoPath(nextRepoPath);
								setSelectedBranches([]);
								setFilter("");
								setError(null);
							}}
						>
							<SelectTrigger size="sm" className="w-[180px]">
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
					)}
				</DialogHeader>

				{/* Body — left: vertical tabs, right: mode content */}
				<Tabs
					orientation="vertical"
					value={mode}
					onValueChange={(v) => {
						setMode(v as CreateMode);
						setSelectedBranches([]);
						setFilter("");
						setError(null);
					}}
					className="flex-1 min-h-0"
				>
					<TabsList variant="line" className="w-[120px] shrink-0 border-r pr-2">
						{tabs.map((tab) => (
							<TabsTrigger key={tab.mode} value={tab.mode}>
								{tab.icon}
								{tab.label}
							</TabsTrigger>
						))}
					</TabsList>
					<div className="flex-1 min-h-0 flex flex-col pl-3">
						{mode === "plain" && (
							<PlainMode
								branchName={selectedBranches[0] ?? ""}
								onBranchNameChange={(name) =>
									setSelectedBranches(name ? [name] : [])
								}
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
								onToggle={toggleBranch}
								selectedBranches={selectedBranches}
							/>
						)}
						{mode === "issue" && selectedRepoPath && (
							<IssueMode
								key={`issue-${selectedRepoPath}`}
								repoPath={selectedRepoPath}
								onToggle={(issue) =>
									toggleBranch(generateIssueBranchName(issue.number))
								}
								selectedBranches={selectedBranches}
								worktreeBranchNames={worktreeBranchNames}
							/>
						)}
						{mode === "notion" && selectedRepoPath && (
							<NotionMode
								key={`notion-${selectedRepoPath}`}
								repoPath={selectedRepoPath}
								onToggle={(task) =>
									toggleBranch(
										task.branch_name || notionTaskToBranchName(task.title),
									)
								}
								selectedBranches={selectedBranches}
								worktreeBranchNames={worktreeBranchNames}
							/>
						)}
					</div>
				</Tabs>

				{/* Footer — error + selected branches left, buttons right */}
				<DialogFooter className="flex-row items-center justify-between gap-2">
					<div className="flex flex-col gap-1 min-w-0">
						{error && <p className="text-xs text-destructive">{error}</p>}
						<div className="flex flex-wrap gap-1 text-xs text-muted-foreground">
							{selectedBranches.map((b) => (
								<code key={b} className="font-mono bg-muted px-1 rounded">
									{b}
								</code>
							))}
						</div>
					</div>
					<div className="flex gap-2 shrink-0">
						<Button variant="outline" onClick={onClose} disabled={creating}>
							Cancel
						</Button>
						<Button
							onClick={handleCreate}
							disabled={
								selectedBranches.length === 0 || !selectedRepoPath || creating
							}
						>
							{creating && <Loader2 className="size-3.5 mr-1 animate-spin" />}
							{creating
								? "Creating..."
								: selectedBranches.length > 1
									? `Create ${selectedBranches.length}`
									: "Create"}
						</Button>
					</div>
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
				<label
					htmlFor="branch-name-input"
					className="text-xs text-muted-foreground"
				>
					Branch name
				</label>
				<Input
					id="branch-name-input"
					value={branchName}
					onChange={(e) => onBranchNameChange(e.target.value)}
					placeholder="feat/my-feature"
					autoFocus
				/>
			</div>
			<div>
				<span className="text-xs text-muted-foreground">Base branch</span>
				<Select value={baseBranch} onValueChange={onBaseBranchChange}>
					<SelectTrigger size="sm" className="w-full" aria-label="Base branch">
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
	onToggle,
	selectedBranches,
}: {
	branches: WorktreeBranch[];
	filter: string;
	onFilterChange: (filter: string) => void;
	onToggle: (name: string) => void;
	selectedBranches: string[];
}) {
	return (
		<div className="flex-1 min-h-0 flex flex-col gap-2">
			<Input
				value={filter}
				onChange={(e) => onFilterChange(e.target.value)}
				placeholder="Filter branches..."
				autoFocus
			/>
			<div className="flex-1 min-h-[120px] overflow-auto">
				<div className="space-y-0.5">
					{branches.map((b) => {
						const isSelected = selectedBranches.includes(b.name);
						return (
							// biome-ignore lint/a11y/useSemanticElements: Checkbox renders as <button> internally, so outer cannot be <button>
							<div
								key={b.name}
								role="button"
								tabIndex={0}
								onClick={() => onToggle(b.name)}
								onKeyDown={(e) => {
									if (e.target !== e.currentTarget) return;
									if (e.key === "Enter" || e.key === " ") {
										e.preventDefault();
										onToggle(b.name);
									}
								}}
								className={`flex w-full items-center gap-2 px-2 py-1.5 text-sm rounded transition-colors cursor-pointer ${
									isSelected ? "bg-muted text-foreground" : "hover:bg-secondary"
								}`}
							>
								<Checkbox
									checked={isSelected}
									onCheckedChange={() => onToggle(b.name)}
									onClick={(e) => e.stopPropagation()}
									onKeyDown={(e) => e.stopPropagation()}
									className="shrink-0"
								/>
								<GitBranch className="size-3.5 text-muted-foreground shrink-0" />
								<span className="truncate">{b.name}</span>
							</div>
						);
					})}
					{branches.length === 0 && (
						<div className="text-xs text-muted-foreground text-center py-4">
							No branches without worktrees
						</div>
					)}
				</div>
			</div>
		</div>
	);
}

function IssueMode({
	repoPath,
	onToggle,
	selectedBranches,
	worktreeBranchNames,
}: {
	repoPath: string;
	onToggle: (issue: IssueInfo) => void;
	selectedBranches: string[];
	worktreeBranchNames: Set<string>;
}) {
	const { issues, loading, refresh } = useIssues(repoPath);
	const [filter, setFilter] = useState("");
	const [labelFilters, setLabelFilters] = useState<string[]>([]);
	const [milestoneFilters, setMilestoneFilters] = useState<string[]>([]);

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
		let result = issues.filter(
			(i) => !worktreeBranchNames.has(generateIssueBranchName(i.number)),
		);
		if (filter) {
			const lower = filter.toLowerCase();
			result = result.filter(
				(i) =>
					i.title.toLowerCase().includes(lower) ||
					String(i.number).includes(lower),
			);
		}
		if (labelFilters.length > 0) {
			// AND: issue must have ALL selected labels
			result = result.filter((i) =>
				labelFilters.every((lf) => i.labels.some((l) => l.name === lf)),
			);
		}
		if (milestoneFilters.length > 0) {
			// OR: issue must belong to ANY selected milestone
			result = result.filter((i) => {
				if (milestoneFilters.includes("__none__")) {
					if (i.milestone === null) return true;
				}
				return i.milestone
					? milestoneFilters.includes(i.milestone.title)
					: false;
			});
		}
		result.sort((a, b) => b.number - a.number);
		return result;
	}, [issues, filter, labelFilters, milestoneFilters, worktreeBranchNames]);

	const milestoneOptions = useMemo(() => {
		const items: { value: string; label: string }[] = [];
		if (allMilestones.hasNone) {
			items.push({ value: "__none__", label: "No milestone" });
		}
		for (const title of allMilestones.titles) {
			items.push({ value: title, label: title });
		}
		return items;
	}, [allMilestones]);

	return (
		<div className="flex-1 min-h-0 flex flex-col gap-1.5">
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
					aria-label="Refresh issues"
					title="Refresh issues"
				>
					<RefreshCw className="size-3.5" />
				</Button>
			</div>
			{(allLabels.length > 0 || milestoneOptions.length > 0) && (
				<div className="flex flex-wrap gap-1">
					{allLabels.length > 0 && (
						<Popover>
							<PopoverTrigger asChild>
								<Button size="sm" variant="outline" className="shrink-0 gap-1">
									Labels
									{labelFilters.length > 0 && (
										<span className="ml-0.5 text-xs bg-foreground/10 text-foreground rounded-full px-1.5 leading-tight">
											{labelFilters.length}
										</span>
									)}
								</Button>
							</PopoverTrigger>
							<PopoverContent
								className="w-[200px] p-0"
								align="start"
								onWheel={(e) => e.stopPropagation()}
							>
								<Command>
									<CommandInput placeholder="Search labels..." />
									<CommandList>
										<CommandEmpty>No labels found.</CommandEmpty>
										<CommandGroup>
											{allLabels.map((label) => {
												const selected = labelFilters.includes(label);
												return (
													<CommandItem
														key={label}
														onSelect={() =>
															setLabelFilters((prev) =>
																prev.includes(label)
																	? prev.filter((l) => l !== label)
																	: [...prev, label],
															)
														}
													>
														<Check
															className={cn(
																"size-3.5",
																selected ? "opacity-100" : "opacity-0",
															)}
														/>
														{label}
													</CommandItem>
												);
											})}
										</CommandGroup>
									</CommandList>
								</Command>
							</PopoverContent>
						</Popover>
					)}
					{milestoneOptions.length > 0 && (
						<Popover>
							<PopoverTrigger asChild>
								<Button size="sm" variant="outline" className="shrink-0 gap-1">
									Milestones
									{milestoneFilters.length > 0 && (
										<span className="ml-0.5 text-xs bg-foreground/10 text-foreground rounded-full px-1.5 leading-tight">
											{milestoneFilters.length}
										</span>
									)}
								</Button>
							</PopoverTrigger>
							<PopoverContent
								className="w-[200px] p-0"
								align="start"
								onWheel={(e) => e.stopPropagation()}
							>
								<Command>
									<CommandInput placeholder="Search milestones..." />
									<CommandList>
										<CommandEmpty>No milestones found.</CommandEmpty>
										<CommandGroup>
											{milestoneOptions.map((ms) => {
												const selected = milestoneFilters.includes(ms.value);
												return (
													<CommandItem
														key={ms.value}
														onSelect={() =>
															setMilestoneFilters((prev) =>
																prev.includes(ms.value)
																	? prev.filter((m) => m !== ms.value)
																	: [...prev, ms.value],
															)
														}
													>
														<Check
															className={cn(
																"size-3.5",
																selected ? "opacity-100" : "opacity-0",
															)}
														/>
														{ms.label}
													</CommandItem>
												);
											})}
										</CommandGroup>
									</CommandList>
								</Command>
							</PopoverContent>
						</Popover>
					)}
				</div>
			)}
			<div className="flex-1 min-h-[120px] overflow-auto">
				{loading ? (
					<div className="flex items-center justify-center py-8">
						<Loader2 className="size-4 text-muted-foreground animate-spin" />
					</div>
				) : (
					<div className="space-y-0.5">
						{filtered.map((issue) => {
							const isSelected = selectedBranches.includes(
								generateIssueBranchName(issue.number),
							);
							return (
								// biome-ignore lint/a11y/useSemanticElements: Checkbox renders as <button> internally, so outer cannot be <button>
								<div
									key={issue.number}
									role="button"
									tabIndex={0}
									onClick={() => onToggle(issue)}
									onKeyDown={(e) => {
										if (e.target !== e.currentTarget) return;
										if (e.key === "Enter" || e.key === " ") {
											e.preventDefault();
											onToggle(issue);
										}
									}}
									className={`flex w-full items-start gap-2 px-2 py-1.5 text-sm rounded transition-colors text-left cursor-pointer ${
										isSelected
											? "bg-muted text-foreground"
											: "hover:bg-secondary"
									}`}
								>
									<Checkbox
										checked={isSelected}
										onCheckedChange={() => onToggle(issue)}
										onClick={(e) => e.stopPropagation()}
										onKeyDown={(e) => e.stopPropagation()}
										className="shrink-0 mt-0.5"
									/>
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
								</div>
							);
						})}
						{filtered.length === 0 && (
							<div className="text-xs text-muted-foreground text-center py-4">
								No issues found
							</div>
						)}
					</div>
				)}
			</div>
		</div>
	);
}

function NotionMode({
	repoPath,
	onToggle,
	selectedBranches,
	worktreeBranchNames,
}: {
	repoPath: string;
	onToggle: (task: NotionTask) => void;
	selectedBranches: string[];
	worktreeBranchNames: Set<string>;
}) {
	const { tasks, loading, loadMore, hasMore, search } =
		useNotionTasks(repoPath);

	const filteredTasks = useMemo(
		() =>
			tasks.filter(
				(t) =>
					!worktreeBranchNames.has(
						t.branch_name || notionTaskToBranchName(t.title),
					),
			),
		[tasks, worktreeBranchNames],
	);
	const { labelOptions } = useNotionLabelOptions(repoPath);
	const [titleFilter, setTitleFilter] = useState("");
	const [labelFilters, setLabelFilters] = useState<Record<string, string[]>>(
		{},
	);

	const handleTitleChange = useCallback(
		(value: string) => {
			setTitleFilter(value);
			search(value, labelFilters);
		},
		[labelFilters, search],
	);

	const toggleLabelFilter = useCallback(
		(propertyName: string, value: string) => {
			setLabelFilters((prev) => {
				const current = prev[propertyName] ?? [];
				const updated = current.includes(value)
					? current.filter((v) => v !== value)
					: [...current, value];
				const newFilters = { ...prev, [propertyName]: updated };
				search(titleFilter, newFilters);
				return newFilters;
			});
		},
		[titleFilter, search],
	);

	return (
		<div className="flex-1 min-h-0 flex flex-col gap-1.5">
			<Input
				value={titleFilter}
				onChange={(e) => handleTitleChange(e.target.value)}
				placeholder="Filter Notion tasks..."
				autoFocus
			/>
			{labelOptions.length > 0 && (
				<div className="flex flex-wrap gap-1">
					{labelOptions.map((opt) => {
						const selected = labelFilters[opt.property_name] ?? [];
						return (
							<Popover key={opt.property_name}>
								<PopoverTrigger asChild>
									<Button
										size="sm"
										variant="outline"
										className="shrink-0 gap-1"
									>
										{opt.property_name}
										{selected.length > 0 && (
											<span className="ml-0.5 text-xs bg-foreground/10 text-foreground rounded-full px-1.5 leading-tight">
												{selected.length}
											</span>
										)}
									</Button>
								</PopoverTrigger>
								<PopoverContent
									className="w-[200px] p-0"
									align="start"
									onWheel={(e) => e.stopPropagation()}
								>
									<Command>
										<CommandInput
											placeholder={`Search ${opt.property_name.toLowerCase()}...`}
										/>
										<CommandList>
											<CommandEmpty>No options found.</CommandEmpty>
											<CommandGroup>
												{opt.options.map((v, i) => {
													const filterValue = opt.option_ids[i] ?? v;
													const isSelected = selected.includes(filterValue);
													return (
														<CommandItem
															key={v}
															onSelect={() =>
																toggleLabelFilter(
																	opt.property_name,
																	filterValue,
																)
															}
														>
															<Check
																className={cn(
																	"size-3.5",
																	isSelected ? "opacity-100" : "opacity-0",
																)}
															/>
															{v}
														</CommandItem>
													);
												})}
											</CommandGroup>
										</CommandList>
									</Command>
								</PopoverContent>
							</Popover>
						);
					})}
				</div>
			)}
			<div className="flex-1 min-h-[120px] overflow-auto">
				{loading ? (
					<div className="flex items-center justify-center py-8">
						<Loader2 className="size-4 text-muted-foreground animate-spin" />
					</div>
				) : (
					<div className="space-y-0.5">
						{filteredTasks.map((task) => {
							const branchName =
								task.branch_name || notionTaskToBranchName(task.title);
							const isSelected = selectedBranches.includes(branchName);
							return (
								// biome-ignore lint/a11y/useSemanticElements: Checkbox renders as <button> internally, so outer cannot be <button>
								<div
									key={task.id}
									role="button"
									tabIndex={0}
									onClick={() => onToggle(task)}
									onKeyDown={(e) => {
										if (e.target !== e.currentTarget) return;
										if (e.key === "Enter" || e.key === " ") {
											e.preventDefault();
											onToggle(task);
										}
									}}
									className={`flex w-full items-start gap-2 px-2 py-1.5 text-sm rounded transition-colors text-left cursor-pointer ${
										isSelected
											? "bg-muted text-foreground"
											: "hover:bg-secondary"
									}`}
								>
									<Checkbox
										checked={isSelected}
										onCheckedChange={() => onToggle(task)}
										onClick={(e) => e.stopPropagation()}
										onKeyDown={(e) => e.stopPropagation()}
										className="shrink-0 mt-0.5"
									/>
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
								</div>
							);
						})}
						{filteredTasks.length === 0 && (
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
			</div>
		</div>
	);
}
