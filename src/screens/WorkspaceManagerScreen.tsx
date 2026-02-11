import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
	CheckCircle2,
	CircleDot,
	FolderOpen,
	GitPullRequest,
	Globe,
	Loader2,
	Plus,
	Settings,
	Terminal,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	ActivityBar,
	type ActivityBarItem,
} from "@/components/layout/ActivityBar";
import { RemotePanel } from "@/components/panels/RemotePanel";
import { SettingsPanel } from "@/components/panels/SettingsPanel";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
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
import { type AppSettings, buildTerminalCommand } from "@/types/settings";

interface WorkspaceManagerScreenProps {
	repoPath: string | null;
	settings: AppSettings;
	providerStatus: ProviderStatus | null;
	initializing?: boolean;
	onSettingsSave: (settings: AppSettings) => void;
	onSelectWorktree: (path: string, branchName?: string) => void;
	onChangeRepo: (path: string | null) => void;
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

export function WorkspaceManagerScreen({
	repoPath,
	settings,
	providerStatus,
	initializing = false,
	onSettingsSave,
	onSelectWorktree,
	onChangeRepo,
}: WorkspaceManagerScreenProps) {
	const [branches, setBranches] = useState<BranchCard[]>([]);
	const [loading, setLoading] = useState(true);
	const [showCreate, setShowCreate] = useState(false);
	const [showSettings, setShowSettings] = useState(false);
	const [activeView, setActiveView] = useState<string | null>(null);
	const [showTerminal, setShowTerminal] = useState(false);
	const [deletingBranch, setDeletingBranch] = useState<BranchCard | null>(null);
	const [openingBranch, setOpeningBranch] = useState<string | null>(null);
	const [baseBranchLabel, setBaseBranchLabel] = useState<string>("");
	const [folderLoading, setFolderLoading] = useState(false);

	const activityBarItems: ActivityBarItem[] = useMemo(
		() => [
			{
				id: "remote",
				icon: <Globe className="size-5" />,
				title: "Remote",
			},
		],
		[],
	);

	const activityBarBottomItems: ActivityBarItem[] = useMemo(
		() => [
			{
				id: "settings",
				icon: <Settings className="size-5" />,
				title: "Settings",
			},
		],
		[],
	);

	const handleActivityItemClick = useCallback((id: string) => {
		setActiveView((prev) => (prev === id ? null : id));
	}, []);

	const repoName = useMemo(
		() => repoPath?.split("/").filter(Boolean).pop() ?? "",
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
		if (!repoPath) {
			setBaseBranchLabel("");
			return;
		}
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
			if (!repoPath) return cards;
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
		if (!repoPath) {
			setBranches([]);
			setLoading(false);
			return;
		}
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
		if (!repoPath) return;
		const unlisten = listen("branch-list-sync", () => {
			refresh();
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, [repoPath, refresh]);

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
		if (!repoPath) return;
		const id = setInterval(() => {
			if (document.visibilityState === "visible") {
				refresh();
			}
		}, 30000);
		return () => clearInterval(id);
	}, [repoPath, refresh]);

	const handleOpenBranch = useCallback(
		async (branch: BranchCard) => {
			if (!repoPath) return;
			setOpeningBranch(branch.name);
			if (branch.worktree_path) {
				onSelectWorktree(branch.worktree_path, branch.name);
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
				onSelectWorktree(entry.path, branch.name);
				setOpeningBranch(null);
			} catch (e) {
				console.error("Failed to create worktree:", e);
				setOpeningBranch(null);
			}
		},
		[repoPath, onSelectWorktree],
	);

	const handleBaseBranchSaved = useCallback(() => {
		refreshBaseBranch();
		refresh();
	}, [refreshBaseBranch, refresh]);

	const handleCreated = useCallback(
		(entry: WorktreeEntry) => {
			setShowCreate(false);
			refresh();
			onSelectWorktree(entry.path, entry.branch);
		},
		[refresh, onSelectWorktree],
	);

	const handleDeleteConfirm = useCallback(
		async (worktreePath: string, force: boolean) => {
			if (!repoPath) return;
			await invoke("kill_ptys_by_worktree", { worktreePath }).catch(() => {});
			await invoke("remove_worktree", { repoPath, worktreePath, force });
			setDeletingBranch(null);
			await refresh();
		},
		[repoPath, refresh],
	);

	const handleSelectFolder = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (!selected) return;
		setFolderLoading(true);
		try {
			const mainPath = await invoke<string>("get_main_repo_path", {
				anyPath: selected,
			});
			onChangeRepo(mainPath);
		} catch {
			onSelectWorktree(selected as string);
		} finally {
			setFolderLoading(false);
		}
	}, [onChangeRepo, onSelectWorktree]);

	if (!repoPath) {
		return (
			<div className="flex flex-col items-center justify-center h-full w-full bg-background text-foreground gap-4">
				{initializing ? (
					<Loader2 className="size-6 text-muted-foreground animate-spin" />
				) : (
					<Button onClick={handleSelectFolder} disabled={folderLoading}>
						{folderLoading ? (
							<Loader2 className="size-4 mr-2 animate-spin" />
						) : (
							<FolderOpen className="size-4 mr-2" />
						)}
						Open Folder
					</Button>
				)}
			</div>
		);
	}

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
		<div className="flex flex-col h-full w-full bg-background text-foreground">
			{/* Header */}
			<div className="flex items-center justify-between h-12 px-4 border-b border-border shrink-0">
				<div className="flex items-center gap-2 min-w-0">
					<h1 className="text-sm font-semibold truncate">{repoName}</h1>
					{baseBranchLabel && (
						<span className="text-xs text-muted-foreground truncate">
							base: {baseBranchLabel}
						</span>
					)}
				</div>
				<div className="flex items-center gap-2">
					<Button
						size="sm"
						variant={showTerminal ? "secondary" : "ghost"}
						onClick={() => setShowTerminal((v) => !v)}
						title="Terminal"
					>
						<Terminal className="size-4" />
					</Button>
					<Button
						size="sm"
						variant="outline"
						onClick={handleSelectFolder}
						disabled={folderLoading}
					>
						{folderLoading ? (
							<Loader2 className="size-4 mr-1 animate-spin" />
						) : (
							<FolderOpen className="size-4 mr-1" />
						)}
						Open
					</Button>
					<Button size="sm" onClick={() => setShowCreate(true)}>
						<Plus className="size-4 mr-1" />
						New
					</Button>
				</div>
			</div>

			{/* Main content: ActivityBar + Sidebar + Kanban + Terminal */}
			<div className="flex flex-1 min-h-0">
				{/* ActivityBar */}
				<ActivityBar
					items={activityBarItems}
					bottomItems={activityBarBottomItems}
					activeItem={activeView ?? undefined}
					onItemClick={handleActivityItemClick}
				/>

				{/* Sidebar */}
				{activeView && (
					<div className="w-64 border-r border-border shrink-0">
						{activeView === "remote" && <RemotePanel rootPath={repoPath} />}
						{activeView === "settings" && (
							<SettingsPanel
								settings={settings}
								onSave={onSettingsSave}
								onOpenRepoSettings={() => setShowSettings(true)}
							/>
						)}
					</div>
				)}

				{/* Kanban board */}
				<div className="flex-1 p-3 min-w-0">
					{loading ? (
						<div className="flex items-center justify-center h-full">
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

				{/* AI Terminal (display:none pattern to preserve PTY session) */}
				<div
					className="border-l border-border shrink-0"
					style={{
						width: showTerminal ? 480 : 0,
						display: showTerminal ? undefined : "none",
					}}
				>
					<TerminalPanel
						cwd={repoPath}
						theme={settings.theme}
						terminalStartupCommand={buildTerminalCommand(settings)}
						sessionKey={`${repoPath}::kanban`}
					/>
				</div>
			</div>

			{/* Status bar */}
			<div className="flex items-center h-6 px-3 bg-primary text-primary-foreground text-xs shrink-0">
				<span className="truncate">{repoPath}</span>
			</div>

			{/* Dialogs */}
			{repoPath && (
				<SettingsDialog
					open={showSettings}
					repoPath={repoPath}
					settings={settings}
					onSave={onSettingsSave}
					onBaseBranchSaved={handleBaseBranchSaved}
					onClose={() => setShowSettings(false)}
				/>
			)}
			{repoPath && (
				<CreateWorktreeDialog
					open={showCreate}
					repoPath={repoPath}
					existingBranches={branches}
					onCreated={handleCreated}
					onCancel={() => setShowCreate(false)}
				/>
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
