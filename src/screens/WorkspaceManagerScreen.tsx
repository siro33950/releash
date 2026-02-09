import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
	CheckCircle2,
	CircleDot,
	FolderOpen,
	Globe,
	Loader2,
	Plus,
	Settings,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { RemotePanel } from "@/components/panels/RemotePanel";
import { Button } from "@/components/ui/button";
import { BaseBranchDialog } from "@/components/workspace/BaseBranchDialog";
import { CreateWorktreeDialog } from "@/components/workspace/CreateWorktreeDialog";
import { DeleteWorktreeDialog } from "@/components/workspace/DeleteWorktreeDialog";
import { KanbanColumn } from "@/components/workspace/KanbanColumn";
import { BranchCard as BranchCardComponent } from "@/components/workspace/WorktreeCard";
import type { BranchCard, WorktreeEntry } from "@/types/git";

interface WorkspaceManagerScreenProps {
	repoPath: string | null;
	onSelectWorktree: (path: string) => void;
	onChangeRepo: (path: string | null) => void;
}

export function WorkspaceManagerScreen({
	repoPath,
	onSelectWorktree,
	onChangeRepo,
}: WorkspaceManagerScreenProps) {
	const [branches, setBranches] = useState<BranchCard[]>([]);
	const [loading, setLoading] = useState(true);
	const [showCreate, setShowCreate] = useState(false);
	const [showBaseBranch, setShowBaseBranch] = useState(false);
	const [showRemote, setShowRemote] = useState(false);
	const [deletingBranch, setDeletingBranch] = useState<BranchCard | null>(null);
	const [baseBranchLabel, setBaseBranchLabel] = useState<string>("");

	const repoName = useMemo(
		() => repoPath?.split("/").filter(Boolean).pop() ?? "",
		[repoPath],
	);

	const { todo, inProgress, done } = useMemo(() => {
		const todo: BranchCard[] = [];
		const inProgress: BranchCard[] = [];
		const done: BranchCard[] = [];
		for (const b of branches) {
			if (b.is_merged) {
				done.push(b);
			} else if (b.worktree_path != null) {
				inProgress.push(b);
			} else {
				todo.push(b);
			}
		}
		return { todo, inProgress, done };
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
			setBranches(cards);
		} catch (e) {
			console.error("Failed to list branches:", e);
		} finally {
			setLoading(false);
		}
		refreshBaseBranch();
	}, [repoPath, refreshBaseBranch]);

	useEffect(() => {
		setLoading(true);
		refresh();
	}, [refresh]);

	useEffect(() => {
		if (!repoPath) return;
		const id = setInterval(() => {
			if (document.visibilityState === "visible") {
				refresh();
			}
		}, 5000);
		return () => clearInterval(id);
	}, [repoPath, refresh]);

	const handleOpenBranch = useCallback(
		async (branch: BranchCard) => {
			if (!repoPath) return;
			if (branch.worktree_path) {
				onSelectWorktree(branch.worktree_path);
				return;
			}
			const parent = repoPath.replace(/\/[^/]+\/?$/, "");
			const repoDir = repoPath.split("/").filter(Boolean).pop();
			const worktreeDir = `${parent}/${repoDir}-worktrees`;
			const dirName = branch.name.replace(/\//g, "-");
			const entry = await invoke<WorktreeEntry>("create_worktree", {
				repoPath,
				worktreePath: `${worktreeDir}/${dirName}`,
				branch: branch.name,
				createBranch: false,
				baseBranch: null,
			});
			onSelectWorktree(entry.path);
		},
		[repoPath, onSelectWorktree],
	);

	const handleBaseBranchSaved = useCallback(() => {
		setShowBaseBranch(false);
		refreshBaseBranch();
		refresh();
	}, [refreshBaseBranch, refresh]);

	const handleCreated = useCallback(
		(entry: WorktreeEntry) => {
			setShowCreate(false);
			refresh();
			onSelectWorktree(entry.path);
		},
		[refresh, onSelectWorktree],
	);

	const handleDeleteConfirm = useCallback(
		async (worktreePath: string, force: boolean) => {
			if (!repoPath) return;
			await invoke("kill_ptys_by_worktree", { worktreePath });
			await invoke("remove_worktree", { repoPath, worktreePath, force });
			setDeletingBranch(null);
			await refresh();
		},
		[repoPath, refresh],
	);

	const handleSelectFolder = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (!selected) return;
		try {
			const mainPath = await invoke<string>("get_main_repo_path", {
				anyPath: selected,
			});
			onChangeRepo(mainPath);
		} catch {
			onSelectWorktree(selected as string);
		}
	}, [onChangeRepo, onSelectWorktree]);

	if (!repoPath) {
		return (
			<div className="flex flex-col items-center justify-center h-screen w-screen bg-background text-foreground gap-4">
				<Button onClick={handleSelectFolder}>
					<FolderOpen className="size-4 mr-2" />
					Open Folder
				</Button>
			</div>
		);
	}

	const renderCards = (cards: BranchCard[]) =>
		cards.map((b) => (
			<BranchCardComponent
				key={b.name}
				branch={b}
				onOpen={() => handleOpenBranch(b)}
				onDelete={setDeletingBranch}
			/>
		));

	return (
		<div className="flex flex-col h-screen w-screen bg-background text-foreground">
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
						onClick={() => setShowBaseBranch(true)}
					>
						<Settings className="size-4" />
					</Button>
					<Button size="sm" variant="outline" onClick={handleSelectFolder}>
						<FolderOpen className="size-4 mr-1" />
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
							<p className="text-muted-foreground">Loading...</p>
						</div>
					) : (
						<div className="grid grid-cols-3 gap-3 h-full">
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
				<BaseBranchDialog
					open={showBaseBranch}
					repoPath={repoPath}
					onSaved={handleBaseBranchSaved}
					onCancel={() => setShowBaseBranch(false)}
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
