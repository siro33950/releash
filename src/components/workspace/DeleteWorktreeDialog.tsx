import { useCallback, useState } from "react";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import type { WorktreeEntry } from "@/types/git";

interface DeleteWorktreeDialogProps {
	open: boolean;
	worktree: WorktreeEntry | null;
	onConfirm: (worktreePath: string, force: boolean) => Promise<void>;
	onCancel: () => void;
}

export function DeleteWorktreeDialog({
	open,
	worktree,
	onConfirm,
	onCancel,
}: DeleteWorktreeDialogProps) {
	const [deleting, setDeleting] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const hasDirty = (worktree?.dirty_count ?? 0) > 0;

	const handleDelete = useCallback(
		async (force: boolean) => {
			if (!worktree) return;
			setDeleting(true);
			setError(null);
			try {
				await onConfirm(worktree.path, force);
			} catch (e) {
				setError(String(e));
				setDeleting(false);
			}
		},
		[worktree, onConfirm],
	);

	const handleOpenChange = useCallback(
		(o: boolean) => {
			if (!o) {
				setError(null);
				setDeleting(false);
				onCancel();
			}
		},
		[onCancel],
	);

	if (!worktree) return null;

	return (
		<AlertDialog open={open} onOpenChange={handleOpenChange}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>Delete Workspace</AlertDialogTitle>
					<AlertDialogDescription>
						Delete workspace &quot;{worktree.name}&quot; on branch &quot;
						{worktree.branch}&quot;?
					</AlertDialogDescription>
				</AlertDialogHeader>
				<div className="grid gap-2 text-sm">
					<div className="text-muted-foreground font-mono text-xs truncate">
						{worktree.path}
					</div>
					{hasDirty && (
						<p className="text-yellow-500">
							This workspace has {worktree.dirty_count} uncommitted change(s).
							Force delete is required.
						</p>
					)}
					{error && <p className="text-destructive">{error}</p>}
				</div>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={onCancel} disabled={deleting}>
						Cancel
					</AlertDialogCancel>
					{hasDirty ? (
						<AlertDialogAction
							variant="destructive"
							onClick={() => handleDelete(true)}
							disabled={deleting}
						>
							{deleting ? "Deleting..." : "Force Delete"}
						</AlertDialogAction>
					) : (
						<AlertDialogAction
							variant="destructive"
							onClick={() => handleDelete(false)}
							disabled={deleting}
						>
							{deleting ? "Deleting..." : "Delete"}
						</AlertDialogAction>
					)}
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
