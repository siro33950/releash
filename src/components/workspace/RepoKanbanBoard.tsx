import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	CircleDot,
	GitPullRequest,
	Loader2,
	Plus,
	RefreshCw,
	Settings,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { CreateWorktreeDialog } from "@/components/workspace/CreateWorktreeDialog";
import { DeleteWorktreeDialog } from "@/components/workspace/DeleteWorktreeDialog";
import { KanbanColumn } from "@/components/workspace/KanbanColumn";
import { SettingsDialog } from "@/components/workspace/SettingsDialog";
import { BranchCard as BranchCardComponent } from "@/components/workspace/WorktreeCard";
import type {
	BranchCard,
	ProviderStatus,
	PrStatus,
	WorktreeEntry,
} from "@/types/git";
import type { AgentStateSync } from "@/types/protocol";

interface RepoKanbanBoardProps {
	repoPath: string;
	providerStatus: ProviderStatus | null;
	onSelectWorktree: (
		path: string,
		branchName?: string,
		repoName?: string,
	) => void;
	onRemove: () => void;
}

function ProviderStatusGuide({ status }: { status: ProviderStatus | null }) {
	if (!status || status === "available" || status === "no_remote") return null;

	let message: string;
	if (typeof status === "object" && "cli_not_found" in status) {
		message = `PR検出を有効にするには ${status.cli_not_found.cli} CLI をインストールしてください`;
	} else if (status === "not_authenticated") {
		message = "gh auth login で認証してください";
	} else if (status === "unsupported_platform") {
		message = "PR検出は現在GitHubに対応しています";
	} else {
		return null;
	}

	return (
		<div className="rounded-lg border border-dashed border-border/50 p-3 text-xs text-muted-foreground">
			{message}
		</div>
	);
}

export function RepoKanbanBoard({
	repoPath,
	providerStatus,
	onSelectWorktree,
	onRemove,
}: RepoKanbanBoardProps) {
	const [branches, setBranches] = useState<BranchCard[]>([]);
	const [loading, setLoading] = useState(true);
	const [showCreate, setShowCreate] = useState(false);
	const [showSettings, setShowSettings] = useState(false);
	const [deletingBranch, setDeletingBranch] = useState<BranchCard | null>(null);
	const [openingBranch, setOpeningBranch] = useState<string | null>(null);
	const [baseBranchLabel, setBaseBranchLabel] = useState<string>("");
	const [collapsed, setCollapsed] = useState(false);
	const [refreshing, setRefreshing] = useState(false);

	const repoName = useMemo(
		() => repoPath.split("/").filter(Boolean).pop() ?? "",
		[repoPath],
	);

	const { todo, inProgress, review, done } = useMemo(() => {
		const todo: BranchCard[] = [];
		const inProgress: BranchCard[] = [];
		const review: BranchCard[] = [];
		const done: BranchCard[] = [];
		for (const b of branches) {
			if (b.is_merged) {
				done.push(b);
			} else if (b.has_pr) {
				review.push(b);
			} else if (b.worktree_path != null) {
				inProgress.push(b);
			} else {
				todo.push(b);
			}
		}
		return { todo, inProgress, review, done };
	}, [branches]);

	const refreshBaseBranch = useCallback(async () => {
		try {
			const base = await invoke<string | null>("get_releash_base", {
				repoPath,
			});
			if (base) {
				setBaseBranchLabel(base);
			} else {
				const detected = await invoke<string>("get_default_branch", {
					repoPath,
				});
				setBaseBranchLabel(`${detected} (auto)`);
			}
		} catch {
			setBaseBranchLabel("");
		}
	}, [repoPath]);

	const enrichWithPrStatus = useCallback(
		async (cards: BranchCard[]): Promise<BranchCard[]> => {
			try {
				const prStatus = await invoke<PrStatus>("get_cached_pr_status", {
					repoPath,
				});
				return cards.map((b) => {
					const pr = prStatus.open_prs[b.name];
					const isMergedViaPr = prStatus.merged_branches.includes(b.name);
					if (pr) {
						return {
							...b,
							has_pr: true,
							pr_number: pr.number,
							pr_url: pr.url,
						};
					}
					if (isMergedViaPr && !b.is_merged) {
						return { ...b, is_merged: true };
					}
					return b;
				});
			} catch {
				return cards;
			}
		},
		[repoPath],
	);

	const refresh = useCallback(async () => {
		try {
			const cards = await invoke<BranchCard[]>("list_branches_with_status", {
				repoPath,
			});
			const enriched = await enrichWithPrStatus(cards);
			const agentStates = await invoke<Record<string, AgentStateSync>>(
				"get_agent_states",
			).catch((): Record<string, AgentStateSync> => ({}));
			setBranches(
				enriched.map((b) => {
					const agent = b.worktree_path
						? agentStates[b.worktree_path]
						: undefined;
					return agent
						? {
								...b,
								agent_state: agent.state,
								agent_state_timestamp: agent.timestamp,
							}
						: b;
				}),
			);
		} catch (e) {
			console.error("Failed to list branches:", e);
		} finally {
			setLoading(false);
		}
		refreshBaseBranch();
	}, [repoPath, refreshBaseBranch, enrichWithPrStatus]);

	useEffect(() => {
		setLoading(true);
		refresh();
	}, [refresh]);

	useEffect(() => {
		const unlisten = listen("branch-list-sync", () => {
			refresh();
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, [refresh]);

	useEffect(() => {
		const unlisten = listen<AgentStateSync>("agent-state-changed", (event) => {
			const { worktree_path, state, timestamp } = event.payload;
			setBranches((prev) =>
				prev.map((b) =>
					b.worktree_path === worktree_path
						? { ...b, agent_state: state, agent_state_timestamp: timestamp }
						: b,
				),
			);
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, []);

	useEffect(() => {
		const id = setInterval(() => {
			if (document.visibilityState === "visible") {
				refresh();
			}
		}, 30000);
		return () => clearInterval(id);
	}, [refresh]);

	const handleOpenBranch = useCallback(
		async (branch: BranchCard) => {
			setOpeningBranch(branch.name);
			if (branch.worktree_path) {
				onSelectWorktree(branch.worktree_path, branch.name, repoName);
				setOpeningBranch(null);
				return;
			}
			const parent = repoPath.replace(/\/[^/]+\/?$/, "");
			const repoDir = repoPath.split("/").filter(Boolean).pop();
			const worktreeDir = `${parent}/${repoDir}-worktrees`;
			const dirName = branch.name.replace(/\//g, "-");
			try {
				const entry = await invoke<WorktreeEntry>("create_worktree", {
					repoPath,
					worktreePath: `${worktreeDir}/${dirName}`,
					branch: branch.name,
					createBranch: false,
					baseBranch: null,
				});
				onSelectWorktree(entry.path, branch.name, repoName);
				setOpeningBranch(null);
			} catch (e) {
				console.error("Failed to create worktree:", e);
				setOpeningBranch(null);
			}
		},
		[repoPath, repoName, onSelectWorktree],
	);

	const handleBaseBranchSaved = useCallback(() => {
		refreshBaseBranch();
		refresh();
	}, [refreshBaseBranch, refresh]);

	const handleCreated = useCallback(
		(entry: WorktreeEntry) => {
			setShowCreate(false);
			refresh();
			onSelectWorktree(entry.path, entry.branch, repoName);
		},
		[refresh, repoName, onSelectWorktree],
	);

	const handleDeleteConfirm = useCallback(
		async (branch: BranchCard, force: boolean) => {
			if (branch.worktree_path) {
				await invoke("kill_ptys_by_worktree", {
					worktreePath: branch.worktree_path,
				}).catch(() => {});
			}
			if (branch.is_merged) {
				await invoke("delete_branch", {
					repoPath,
					branchName: branch.name,
					force,
				});
			} else if (branch.worktree_path) {
				await invoke("remove_worktree", {
					repoPath,
					worktreePath: branch.worktree_path,
					force,
				});
			}
			await refresh();
			setDeletingBranch(null);
		},
		[repoPath, refresh],
	);

	const handleRefresh = useCallback(async () => {
		setRefreshing(true);
		try {
			await refresh();
		} finally {
			setRefreshing(false);
		}
	}, [refresh]);

	const renderCards = (cards: BranchCard[]) =>
		cards.map((b) => (
			<BranchCardComponent
				key={b.name}
				branch={b}
				opening={openingBranch === b.name}
				onOpen={() => handleOpenBranch(b)}
				onDelete={setDeletingBranch}
			/>
		));

	return (
		<div className="border-b border-border">
			{/* Repo header */}
			<div className="flex items-center justify-between h-10 px-3 bg-sidebar/50">
				<div className="flex items-center gap-2 min-w-0">
					<button
						type="button"
						onClick={() => setCollapsed((prev) => !prev)}
						className="p-0.5 rounded hover:bg-muted transition-colors"
					>
						{collapsed ? (
							<ChevronRight className="size-4" />
						) : (
							<ChevronDown className="size-4" />
						)}
					</button>
					<span className="text-sm font-semibold truncate">{repoName}</span>
					{baseBranchLabel && (
						<span className="text-xs text-muted-foreground truncate">
							base: {baseBranchLabel}
						</span>
					)}
				</div>
				<div className="flex items-center gap-1">
					<Button
						size="icon"
						variant="ghost"
						className="size-7"
						onClick={handleRefresh}
						disabled={refreshing}
						title="Refresh"
					>
						<RefreshCw
							className={`size-3.5 ${refreshing ? "animate-spin" : ""}`}
						/>
					</Button>
					<Button
						size="icon"
						variant="ghost"
						className="size-7"
						onClick={() => setShowCreate(true)}
						title="New worktree"
					>
						<Plus className="size-3.5" />
					</Button>
					<Button
						size="icon"
						variant="ghost"
						className="size-7"
						onClick={() => setShowSettings(true)}
						title="Repository settings"
					>
						<Settings className="size-3.5" />
					</Button>
					<Button
						size="icon"
						variant="ghost"
						className="size-7"
						onClick={onRemove}
						title="Remove repository"
					>
						<X className="size-3.5" />
					</Button>
				</div>
			</div>

			{/* Kanban board */}
			{!collapsed && (
				<div className="p-3 min-w-0">
					{loading ? (
						<div className="flex items-center justify-center py-8">
							<Loader2 className="size-5 text-muted-foreground animate-spin" />
						</div>
					) : (
						<div className="flex gap-3 h-full overflow-x-auto">
							<KanbanColumn
								icon={<CircleDot className="size-3.5 text-muted-foreground" />}
								title="Todo"
								count={todo.length}
							>
								{renderCards(todo)}
							</KanbanColumn>
							<KanbanColumn
								icon={<Loader2 className="size-3.5 text-blue-500" />}
								title="In Progress"
								count={inProgress.length}
							>
								{renderCards(inProgress)}
							</KanbanColumn>
							<KanbanColumn
								icon={<GitPullRequest className="size-3.5 text-purple-500" />}
								title="Review"
								count={review.length}
							>
								{renderCards(review)}
								<ProviderStatusGuide status={providerStatus} />
							</KanbanColumn>
							<KanbanColumn
								icon={<CheckCircle2 className="size-3.5 text-green-500" />}
								title="Done"
								count={done.length}
							>
								{renderCards(done)}
							</KanbanColumn>
						</div>
					)}
				</div>
			)}

			{/* Dialogs */}
			<SettingsDialog
				open={showSettings}
				repoPath={repoPath}
				onBaseBranchSaved={handleBaseBranchSaved}
				onClose={() => setShowSettings(false)}
			/>
			<CreateWorktreeDialog
				open={showCreate}
				repoPath={repoPath}
				existingBranches={branches}
				onCreated={handleCreated}
				onCancel={() => setShowCreate(false)}
			/>
			<DeleteWorktreeDialog
				open={!!deletingBranch}
				branch={deletingBranch}
				onConfirm={handleDeleteConfirm}
				onCancel={() => setDeletingBranch(null)}
			/>
		</div>
	);
}
