import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useMemo, useState } from "react";
import { CreateWorktreeDialog } from "@/components/workspace/CreateWorktreeDialog";
import { DeleteWorktreeDialog } from "@/components/workspace/DeleteWorktreeDialog";
import { NewWorktreeCard } from "@/components/workspace/NewWorktreeCard";
import { WorktreeCard } from "@/components/workspace/WorktreeCard";
import { useWorktrees } from "@/hooks/useWorktrees";
import type { WorktreeEntry } from "@/types/git";

const WORKTREE_ROOT_KEY_PREFIX = "releash-worktree-root:";

function getWorktreeRoot(repoPath: string): string {
	const key = `${WORKTREE_ROOT_KEY_PREFIX}${repoPath}`;
	const stored = localStorage.getItem(key);
	if (stored) return stored;
	const parent = repoPath.replace(/\/[^/]+\/?$/, "");
	const repoName = repoPath.split("/").filter(Boolean).pop() ?? "repo";
	return `${parent}/${repoName}-worktrees`;
}

function setWorktreeRoot(repoPath: string, root: string): void {
	const key = `${WORKTREE_ROOT_KEY_PREFIX}${repoPath}`;
	localStorage.setItem(key, root);
}

interface WorkspaceManagerScreenProps {
	repoPath: string | null;
	onSelectWorktree: (path: string) => void;
}

export function WorkspaceManagerScreen({
	repoPath,
	onSelectWorktree,
}: WorkspaceManagerScreenProps) {
	const { worktrees, loading, refresh, removeWorktree } =
		useWorktrees(repoPath);
	const [showCreate, setShowCreate] = useState(false);
	const [deletingWorktree, setDeletingWorktree] =
		useState<WorktreeEntry | null>(null);

	const worktreeRoot = useMemo(
		() => (repoPath ? getWorktreeRoot(repoPath) : ""),
		[repoPath],
	);

	const repoName = useMemo(
		() => repoPath?.split("/").filter(Boolean).pop() ?? "",
		[repoPath],
	);

	const handleChangeRoot = useCallback(async () => {
		if (!repoPath) return;
		const selected = await open({ directory: true });
		if (selected) {
			setWorktreeRoot(repoPath, selected);
		}
	}, [repoPath]);

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
			await removeWorktree(worktreePath, force);
			setDeletingWorktree(null);
		},
		[removeWorktree],
	);

	if (!repoPath) {
		return (
			<div className="flex items-center justify-center h-screen w-screen bg-background text-foreground">
				<p className="text-muted-foreground">
					{loading ? "Loading..." : "No git repository detected"}
				</p>
			</div>
		);
	}

	return (
		<div className="flex flex-col h-screen w-screen bg-background text-foreground">
			{/* Header */}
			<div className="flex items-center justify-between h-12 px-4 border-b border-border shrink-0">
				<h1 className="text-sm font-semibold truncate">{repoName}</h1>
			</div>

			{/* Worktree Root bar */}
			<div className="flex items-center gap-2 h-9 px-4 border-b border-border text-xs text-muted-foreground shrink-0">
				<span className="shrink-0">Root:</span>
				<span className="truncate font-mono">{worktreeRoot}</span>
				<button
					type="button"
					onClick={handleChangeRoot}
					className="shrink-0 px-2 py-0.5 rounded hover:bg-accent transition-colors"
				>
					Change
				</button>
			</div>

			{/* Card grid */}
			<div className="flex-1 overflow-y-auto p-4">
				{loading ? (
					<div className="flex items-center justify-center h-full">
						<p className="text-muted-foreground">Loading...</p>
					</div>
				) : (
					<div className="grid grid-cols-2 lg:grid-cols-3 gap-4">
						{worktrees.map((wt) => (
							<WorktreeCard
								key={wt.path}
								worktree={wt}
								repoPath={repoPath}
								onOpen={onSelectWorktree}
								onDelete={setDeletingWorktree}
							/>
						))}
						<NewWorktreeCard onClick={() => setShowCreate(true)} />
					</div>
				)}
			</div>

			{/* Status bar */}
			<div className="flex items-center h-6 px-3 bg-primary text-primary-foreground text-xs shrink-0">
				<span className="truncate">{repoPath}</span>
			</div>

			{/* Dialogs */}
			{repoPath && (
				<CreateWorktreeDialog
					open={showCreate}
					repoPath={repoPath}
					worktreeRoot={worktreeRoot}
					existingWorktrees={worktrees}
					onCreated={handleCreated}
					onCancel={() => setShowCreate(false)}
				/>
			)}
			<DeleteWorktreeDialog
				open={!!deletingWorktree}
				worktree={deletingWorktree}
				onConfirm={handleDeleteConfirm}
				onCancel={() => setDeletingWorktree(null)}
			/>
		</div>
	);
}
