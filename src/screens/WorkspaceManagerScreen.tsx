import { invoke } from "@tauri-apps/api/core";
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
	X,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { RemotePanel } from "@/components/panels/RemotePanel";
import { Button } from "@/components/ui/button";
import { CreateWorktreeDialog } from "@/components/workspace/CreateWorktreeDialog";
import { DeleteWorktreeDialog } from "@/components/workspace/DeleteWorktreeDialog";
import { KanbanColumn } from "@/components/workspace/KanbanColumn";
import { SettingsDialog } from "@/components/workspace/SettingsDialog";
import { BranchCard as BranchCardComponent } from "@/components/workspace/WorktreeCard";
import { useKanbanBoard } from "@/hooks/useKanbanBoard";
import type { BranchCard, ProviderStatus, WorktreeEntry } from "@/types/git";
import type { AppSettings } from "@/types/settings";

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
	const {
		branches,
		loading,
		baseBranchLabel,
		todo,
		inProgress,
		review,
		done,
		refresh,
		refreshBaseBranch,
	} = useKanbanBoard(repoPath);

	const [showCreate, setShowCreate] = useState(false);
	const [showSettings, setShowSettings] = useState(false);
	const [showRemote, setShowRemote] = useState(false);
	const [deletingBranch, setDeletingBranch] = useState<BranchCard | null>(null);
	const [openingBranch, setOpeningBranch] = useState<string | null>(null);
	const [folderLoading, setFolderLoading] = useState(false);

	const repoName = useMemo(
		() => repoPath?.split("/").filter(Boolean).pop() ?? "",
		[repoPath],
	);

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
		async (branch: BranchCard, force: boolean) => {
			if (!repoPath) return;
			try {
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
				} else {
					await invoke("remove_worktree", {
						repoPath,
						worktreePath: branch.worktree_path,
						force,
					});
				}
				await refresh();
				setDeletingBranch(null);
			} catch (e) {
				console.error("Failed to delete branch/worktree:", e);
				throw e;
			}
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
				onOpen={handleOpenBranch}
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
						variant={showRemote ? "secondary" : "ghost"}
						onClick={() => setShowRemote((v) => !v)}
						title="Remote"
					>
						<Globe className="size-4" />
					</Button>
					<Button
						size="sm"
						variant="ghost"
						onClick={() => setShowSettings(true)}
					>
						<Settings className="size-4" />
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

			{/* Kanban board + Remote side panel */}
			<div className="flex flex-1 min-h-0">
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
				{showRemote && (
					<div className="w-72 border-l border-border shrink-0 relative">
						<button
							type="button"
							className="absolute top-1 right-1 p-0.5 rounded hover:bg-muted z-10"
							onClick={() => setShowRemote(false)}
						>
							<X className="size-3.5 text-muted-foreground" />
						</button>
						<RemotePanel rootPath={repoPath} />
					</div>
				)}
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
