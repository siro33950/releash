import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
	ChevronDown,
	ChevronRight,
	Filter,
	Globe,
	LayoutList,
	Loader2,
	Plus,
	RefreshCw,
	Settings,
	Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AgentStateIcon } from "@/components/ui/agent-state-icon";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuLabel,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import {
	computeStatus,
	useWorktreeList,
	type WorktreeStatus,
} from "@/hooks/useWorktreeList";
import { trackEvent } from "@/lib/telemetry";
import type { WorktreeBranch } from "@/types/git";
import { CreateWorktreeModal } from "./CreateWorktreeModal";
import { DeleteWorktreeDialog } from "./DeleteWorktreeDialog";

type GroupMode = "repository" | "status";

interface WorktreeData {
	branches: WorktreeBranch[];
	loading: boolean;
	refresh: (options?: { silent?: boolean }) => Promise<void>;
}

const noopRefresh = async (_options?: { silent?: boolean }) => {};

interface WorkspaceListProps {
	repoPaths: string[];
	selectedRootPath: string | null;
	onSelectWorktree: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
	) => void;
	onAddRepo: () => void;
	onShowRemote: () => void;
	onShowSettings: () => void;
}

const STATUS_ORDER: WorktreeStatus[] = [
	"backlog",
	"in_progress",
	"review",
	"done",
];
const STATUS_LABELS: Record<WorktreeStatus, string> = {
	backlog: "Backlog",
	in_progress: "In Progress",
	review: "Review",
	done: "Done",
};

function RepoWorktreeSectionView({
	repoPath,
	branches,
	loading,
	refresh,
	selectedRootPath,
	onSelectWorktree,
	groupMode,
	filterStatus,
	statusFilter,
}: {
	repoPath: string;
	branches: WorktreeBranch[];
	loading: boolean;
	refresh: (options?: { silent?: boolean }) => Promise<void>;
	selectedRootPath: string | null;
	onSelectWorktree: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
	) => void;
	groupMode: GroupMode;
	filterStatus?: WorktreeStatus;
	statusFilter?: string;
}) {
	const [collapsed, setCollapsed] = useState(false);
	const [deletingBranch, setDeletingBranch] = useState<WorktreeBranch | null>(
		null,
	);
	const [refreshing, setRefreshing] = useState(false);

	const repoName = useMemo(
		() => repoPath.split("/").filter(Boolean).pop() ?? "",
		[repoPath],
	);

	const handleRefresh = useCallback(async () => {
		setRefreshing(true);
		try {
			await refresh();
		} finally {
			setRefreshing(false);
		}
	}, [refresh]);

	const handleDeleteConfirm = useCallback(
		async (branch: WorktreeBranch, force: boolean) => {
			try {
				if (branch.worktree_path) {
					await invoke("kill_ptys_by_worktree", {
						worktreePath: branch.worktree_path,
					}).catch(() => {});
					await invoke("remove_worktree", {
						repoPath,
						worktreePath: branch.worktree_path,
						force,
					});
					trackEvent("worktree_removed");
				} else if (branch.is_merged) {
					await invoke("delete_branch", {
						repoPath,
						branchName: branch.name,
						force,
					});
				}
				await refresh();
			} finally {
				setDeletingBranch(null);
			}
		},
		[repoPath, refresh],
	);

	const handleOpenBranch = useCallback(
		(branch: WorktreeBranch) => {
			if (branch.worktree_path) {
				onSelectWorktree(branch.worktree_path, branch.name, repoName);
			}
		},
		[repoName, onSelectWorktree],
	);

	const filteredBranches = useMemo(() => {
		const filtered =
			!statusFilter || statusFilter === "all"
				? branches
				: branches.filter((b) => computeStatus(b) === statusFilter);
		return [...filtered].sort(
			(a, b) =>
				STATUS_ORDER.indexOf(computeStatus(a)) -
				STATUS_ORDER.indexOf(computeStatus(b)),
		);
	}, [branches, statusFilter]);

	const groupedByStatus = useMemo(() => {
		const groups: Record<WorktreeStatus, WorktreeBranch[]> = {
			backlog: [],
			in_progress: [],
			review: [],
			done: [],
		};
		for (const b of filteredBranches) {
			groups[computeStatus(b)].push(b);
		}
		return groups;
	}, [filteredBranches]);

	const renderItem = (branch: WorktreeBranch) => {
		const isSelected = branch.worktree_path === selectedRootPath;
		const hasWorktree = branch.worktree_path != null;
		const canDelete = !branch.is_default && (hasWorktree || branch.is_merged);
		const status = computeStatus(branch);

		// 2行目テキスト部分を組み立て
		const infoParts: string[] = [];
		if (groupMode === "status") {
			infoParts.push(repoName);
		} else {
			infoParts.push(STATUS_LABELS[status]);
		}
		if (branch.is_merged) infoParts.push("merged");
		if (hasWorktree && branch.dirty_count > 0)
			infoParts.push(`${branch.dirty_count} changed`);

		return (
			// biome-ignore lint/a11y/useSemanticElements: <button> cannot nest <button> (PR link, delete btn)
			<div
				key={branch.name}
				role="button"
				tabIndex={0}
				data-testid={`worktree-item-${branch.name}`}
				onClick={() => handleOpenBranch(branch)}
				onKeyDown={(e) => {
					if (e.target !== e.currentTarget) return;
					if (e.key === "Enter" || e.key === " ") {
						e.preventDefault();
						handleOpenBranch(branch);
					}
				}}
				className={`group relative flex items-start gap-1.5 w-full px-1.5 py-1.5 text-left rounded cursor-pointer transition-colors outline-none focus-visible:ring-1 focus-visible:ring-ring ${
					isSelected
						? "bg-foreground/10 text-foreground"
						: hasWorktree
							? "text-foreground hover:bg-foreground/5"
							: "text-muted-foreground hover:bg-foreground/5"
				}`}
			>
				<div className="flex flex-col gap-1 min-w-0 flex-1">
					{/* Row 1: icon + name + diff stats */}
					<div className="flex items-center gap-1.5 min-w-0">
						<AgentStateIcon state={branch.agent_state} />
						<span className="text-xs font-medium truncate flex-1">
							{branch.name}
						</span>
						{branch.ahead > 0 && (
							<span className="shrink-0 text-[11px] font-mono text-success/70">
								+{branch.ahead}
							</span>
						)}
						{branch.behind > 0 && (
							<span className="shrink-0 text-[11px] font-mono text-destructive/70">
								-{branch.behind}
							</span>
						)}
					</div>
					{/* Row 2: secondary info */}
					<div className="flex items-center gap-1.5 pl-[20px] min-w-0 text-[11px] text-muted-foreground">
						{infoParts.length > 0 && (
							<span className="truncate">{infoParts.join(" · ")}</span>
						)}
						{branch.has_pr && branch.pr_url && branch.pr_number != null && (
							<button
								type="button"
								className="shrink-0 ml-auto text-[11px] text-muted-foreground hover:text-foreground transition-colors"
								onClick={(e) => {
									e.stopPropagation();
									openUrl(branch.pr_url as string);
								}}
							>
								#{branch.pr_number}
							</button>
						)}
					</div>
				</div>
				{canDelete && (
					<Button
						size="icon-xs"
						variant="ghost"
						className="absolute top-0.5 right-0.5 hidden group-hover:flex group-focus-within:flex size-4"
						onClick={(e) => {
							e.stopPropagation();
							setDeletingBranch(branch);
						}}
						aria-label={`Delete ${branch.name}`}
					>
						<Trash2 className="size-2.5 text-muted-foreground" />
					</Button>
				)}
			</div>
		);
	};

	if (groupMode === "status" && filterStatus) {
		const items = groupedByStatus[filterStatus];
		return (
			<>
				{loading && (
					<div className="flex items-center justify-center py-4">
						<Loader2 className="size-4 text-muted-foreground animate-spin" />
					</div>
				)}
				{!loading && items.length > 0 && <div>{items.map(renderItem)}</div>}
				<DeleteWorktreeDialog
					open={!!deletingBranch}
					branch={deletingBranch}
					onConfirm={handleDeleteConfirm}
					onCancel={() => setDeletingBranch(null)}
				/>
			</>
		);
	}

	return (
		<div>
			<div className="flex items-center gap-1.5 w-full px-2 py-1 text-xs font-semibold text-muted-foreground">
				<button
					type="button"
					onClick={() => setCollapsed((prev) => !prev)}
					className="flex items-center gap-1.5 flex-1 min-w-0 hover:text-foreground transition-colors"
				>
					{collapsed ? (
						<ChevronRight className="size-3.5" />
					) : (
						<ChevronDown className="size-3.5" />
					)}
					<span className="truncate">{repoName}</span>
					<span className="ml-auto text-[10px]">{filteredBranches.length}</span>
				</button>
				<Button
					size="icon-xs"
					variant="ghost"
					className="size-5 ml-0.5"
					onClick={handleRefresh}
					disabled={refreshing}
					aria-label={`Refresh ${repoName}`}
					title={`Refresh ${repoName}`}
				>
					<RefreshCw className={`size-3 ${refreshing ? "animate-spin" : ""}`} />
				</Button>
			</div>
			{!collapsed && (
				<div className="pl-2">
					{loading && (
						<div className="flex items-center justify-center py-4">
							<Loader2 className="size-4 text-muted-foreground animate-spin" />
						</div>
					)}
					{!loading && filteredBranches.map(renderItem)}
				</div>
			)}
			<DeleteWorktreeDialog
				open={!!deletingBranch}
				branch={deletingBranch}
				onConfirm={handleDeleteConfirm}
				onCancel={() => setDeletingBranch(null)}
			/>
		</div>
	);
}

function RepoWorktreeSection({
	repoPath,
	selectedRootPath,
	onSelectWorktree,
	groupMode,
	filterStatus,
	statusFilter,
}: {
	repoPath: string;
	selectedRootPath: string | null;
	onSelectWorktree: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
	) => void;
	groupMode: GroupMode;
	filterStatus?: WorktreeStatus;
	statusFilter?: string;
}) {
	const { branches, loading, refresh } = useWorktreeList(repoPath);
	return (
		<RepoWorktreeSectionView
			repoPath={repoPath}
			branches={branches}
			loading={loading}
			refresh={refresh}
			selectedRootPath={selectedRootPath}
			onSelectWorktree={onSelectWorktree}
			groupMode={groupMode}
			filterStatus={filterStatus}
			statusFilter={statusFilter}
		/>
	);
}

function RepoWorktreeFetcher({
	repoPath,
	onData,
}: {
	repoPath: string;
	onData: (repoPath: string, data: WorktreeData) => void;
}) {
	const { branches, loading, refresh } = useWorktreeList(repoPath);
	const onDataRef = useRef(onData);
	onDataRef.current = onData;

	useEffect(() => {
		onDataRef.current(repoPath, { branches, loading, refresh });
	}, [repoPath, branches, loading, refresh]);

	return null;
}

export function WorkspaceList({
	repoPaths,
	selectedRootPath,
	onSelectWorktree,
	onAddRepo,
	onShowRemote,
	onShowSettings,
}: WorkspaceListProps) {
	const [groupMode, setGroupMode] = useState<GroupMode>("repository");
	const [showCreate, setShowCreate] = useState(false);
	const [statusFilter, setStatusFilter] = useState("all");
	const [repoFilter, setRepoFilter] = useState("all");

	const filteredRepoPaths = useMemo(
		() =>
			repoFilter === "all"
				? repoPaths
				: repoPaths.filter((p) => p === repoFilter),
		[repoPaths, repoFilter],
	);

	const repoNames = useMemo(
		() =>
			repoPaths.map((p) => ({
				path: p,
				name: p.split("/").filter(Boolean).pop() ?? "",
			})),
		[repoPaths],
	);

	return (
		<div className="flex flex-col h-full">
			{/* Header */}
			<div className="flex items-center justify-between h-9 px-2 shrink-0">
				<span className="text-xs font-semibold tracking-wide text-muted-foreground">
					Workspaces
				</span>
				<div className="flex items-center gap-0.5">
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button
								size="icon-xs"
								variant="ghost"
								className="size-5"
								aria-label="グループ"
								title="Group by"
							>
								<LayoutList className="size-3" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end" className="p-2">
							<DropdownMenuLabel className="p-0 pb-1 text-xs font-normal text-muted-foreground">
								Group by
							</DropdownMenuLabel>
							<Select
								value={groupMode}
								onValueChange={(v) => setGroupMode(v as GroupMode)}
							>
								<SelectTrigger className="h-6">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectGroup>
										<SelectItem value="repository">Repo</SelectItem>
										<SelectItem value="status">Status</SelectItem>
									</SelectGroup>
								</SelectContent>
							</Select>
						</DropdownMenuContent>
					</DropdownMenu>
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button
								size="icon-xs"
								variant="ghost"
								className="size-5"
								aria-label="フィルター"
								title="Filter"
							>
								<Filter className="size-3" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end" className="p-2">
							<DropdownMenuLabel className="p-0 pb-1 text-xs font-normal text-muted-foreground">
								Filter
							</DropdownMenuLabel>
							<Select
								value={groupMode === "repository" ? statusFilter : repoFilter}
								onValueChange={(v) => {
									if (groupMode === "repository") {
										setStatusFilter(v);
									} else {
										setRepoFilter(v);
									}
								}}
							>
								<SelectTrigger className="h-6">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectGroup>
										<SelectItem value="all">All</SelectItem>
										{groupMode === "repository"
											? STATUS_ORDER.map((s) => (
													<SelectItem key={s} value={s}>
														{STATUS_LABELS[s]}
													</SelectItem>
												))
											: repoNames.map((r) => (
													<SelectItem key={r.path} value={r.path}>
														{r.name}
													</SelectItem>
												))}
									</SelectGroup>
								</SelectContent>
							</Select>
						</DropdownMenuContent>
					</DropdownMenu>
					<Button
						size="icon-xs"
						variant="ghost"
						className="size-5"
						onClick={() => setShowCreate(true)}
						title="Add worktree"
						aria-label="ワークツリーを追加"
					>
						<Plus className="size-3" />
					</Button>
				</div>
			</div>

			{/* Worktree List */}
			<div className="flex-1 overflow-y-auto px-0.5 py-0.5 space-y-0.5">
				{groupMode === "status" ? (
					<StatusGroupedView
						repoPaths={filteredRepoPaths}
						selectedRootPath={selectedRootPath}
						onSelectWorktree={onSelectWorktree}
					/>
				) : (
					repoPaths.map((repoPath) => (
						<RepoWorktreeSection
							key={repoPath}
							repoPath={repoPath}
							selectedRootPath={selectedRootPath}
							onSelectWorktree={onSelectWorktree}
							groupMode="repository"
							statusFilter={statusFilter}
						/>
					))
				)}
				{repoPaths.length === 0 && (
					<div className="text-xs text-muted-foreground text-center py-8">
						No repositories
					</div>
				)}
			</div>

			{/* Bottom buttons */}
			<div className="flex items-center justify-between h-[36px] px-2 border-t border-border shrink-0">
				<Button
					size="sm"
					variant="ghost"
					className="h-7 px-2 text-xs"
					onClick={onAddRepo}
				>
					<Plus className="size-3.5 mr-1" />
					Add Repository
				</Button>
				<div className="flex items-center gap-0.5">
					<Button
						size="icon"
						variant="ghost"
						className="size-7"
						onClick={onShowRemote}
						title="Remote"
					>
						<Globe className="size-3.5" />
					</Button>
					<Button
						size="icon"
						variant="ghost"
						className="size-7"
						onClick={onShowSettings}
						title="Settings"
					>
						<Settings className="size-3.5" />
					</Button>
				</div>
			</div>

			{/* Create Worktree Modal */}
			{showCreate && repoPaths.length > 0 && (
				<CreateWorktreeModal
					open={showCreate}
					repoPaths={repoPaths}
					onCreated={(rootPath, branchName, repoName) => {
						setShowCreate(false);
						emit("branch-list-sync");
						onSelectWorktree(rootPath, branchName, repoName);
					}}
					onClose={() => setShowCreate(false)}
				/>
			)}
		</div>
	);
}

function StatusGroupSection({
	status,
	repoPaths,
	worktreeDataMap,
	selectedRootPath,
	onSelectWorktree,
}: {
	status: WorktreeStatus;
	repoPaths: string[];
	worktreeDataMap: Map<string, WorktreeData>;
	selectedRootPath: string | null;
	onSelectWorktree: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
	) => void;
}) {
	const [collapsed, setCollapsed] = useState(false);

	return (
		<div>
			<button
				type="button"
				onClick={() => setCollapsed((prev) => !prev)}
				className="flex items-center gap-1.5 w-full px-2 py-1 text-xs font-semibold text-muted-foreground hover:text-foreground transition-colors"
			>
				{collapsed ? (
					<ChevronRight className="size-3.5" />
				) : (
					<ChevronDown className="size-3.5" />
				)}
				<span>{STATUS_LABELS[status]}</span>
			</button>
			{!collapsed && (
				<div className="pl-2">
					{repoPaths.map((repoPath) => {
						const data = worktreeDataMap.get(repoPath);
						return (
							<RepoWorktreeSectionView
								key={repoPath}
								repoPath={repoPath}
								branches={data?.branches ?? []}
								loading={data?.loading ?? true}
								refresh={data?.refresh ?? noopRefresh}
								selectedRootPath={selectedRootPath}
								onSelectWorktree={onSelectWorktree}
								groupMode="status"
								filterStatus={status}
							/>
						);
					})}
				</div>
			)}
		</div>
	);
}

function StatusGroupedView({
	repoPaths,
	selectedRootPath,
	onSelectWorktree,
}: {
	repoPaths: string[];
	selectedRootPath: string | null;
	onSelectWorktree: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
	) => void;
}) {
	const [worktreeDataMap, setWorktreeDataMap] = useState<
		Map<string, WorktreeData>
	>(() => new Map());

	const handleData = useCallback((repoPath: string, data: WorktreeData) => {
		setWorktreeDataMap((prev) => {
			const next = new Map(prev);
			next.set(repoPath, data);
			return next;
		});
	}, []);

	return (
		<>
			{repoPaths.map((repoPath) => (
				<RepoWorktreeFetcher
					key={repoPath}
					repoPath={repoPath}
					onData={handleData}
				/>
			))}
			{STATUS_ORDER.map((status) => (
				<StatusGroupSection
					key={status}
					status={status}
					repoPaths={repoPaths}
					worktreeDataMap={worktreeDataMap}
					selectedRootPath={selectedRootPath}
					onSelectWorktree={onSelectWorktree}
				/>
			))}
		</>
	);
}
