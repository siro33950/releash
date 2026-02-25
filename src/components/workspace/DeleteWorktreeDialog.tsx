import { Loader2 } from "lucide-react";
import { useCallback, useRef, useState } from "react";
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
import type { WorktreeBranch } from "@/types/git";

interface DeleteWorktreeDialogProps {
	open: boolean;
	branch: WorktreeBranch | null;
	onConfirm: (branch: WorktreeBranch, force: boolean) => Promise<void>;
	onCancel: () => void;
}

export function DeleteWorktreeDialog({
	open,
	branch,
	onConfirm,
	onCancel,
}: DeleteWorktreeDialogProps) {
	const [deleting, setDeleting] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const deletingRef = useRef(false);

	const hasDirty = (branch?.dirty_count ?? 0) > 0;

	const handleDelete = useCallback(
		async (force: boolean) => {
			if (!branch || (!branch.worktree_path && !branch.is_merged)) return;
			setDeleting(true);
			deletingRef.current = true;
			setError(null);
			try {
				await onConfirm(branch, force);
			} catch (e) {
				setError(String(e));
			} finally {
				setDeleting(false);
				deletingRef.current = false;
			}
		},
		[branch, onConfirm],
	);

	const handleOpenChange = useCallback(
		(o: boolean) => {
			if (!o) {
				if (deletingRef.current) return;
				setError(null);
				onCancel();
			}
		},
		[onCancel],
	);

	if (!branch) return null;

	const isMergedOnly = branch.is_merged && !branch.worktree_path;
	const isMergedWithWorktree = branch.is_merged && !!branch.worktree_path;

	const title = branch.is_merged ? "Delete Branch" : "Delete Workspace";
	const description = isMergedOnly
		? `Delete merged branch "${branch.name}"?`
		: isMergedWithWorktree
			? `Delete workspace and branch "${branch.name}"?`
			: `Delete workspace for branch "${branch.name}"?`;

	return (
		<AlertDialog open={open} onOpenChange={handleOpenChange}>
			<AlertDialogContent
				onEscapeKeyDown={(e) => {
					if (deleting) e.preventDefault();
				}}
			>
				<AlertDialogHeader>
					<AlertDialogTitle>{title}</AlertDialogTitle>
					<AlertDialogDescription>{description}</AlertDialogDescription>
				</AlertDialogHeader>
				<div className="grid gap-2 text-sm">
					{branch.worktree_path && (
						<div className="text-muted-foreground font-mono text-xs truncate">
							{branch.worktree_path}
						</div>
					)}
					{hasDirty && (
						<p className="text-warning">
							This workspace has {branch.dirty_count} uncommitted change(s).
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
							{deleting && <Loader2 className="size-3.5 mr-1 animate-spin" />}
							{deleting ? "Deleting..." : "Force Delete"}
						</AlertDialogAction>
					) : (
						<AlertDialogAction
							variant="destructive"
							onClick={() => handleDelete(false)}
							disabled={deleting}
						>
							{deleting && <Loader2 className="size-3.5 mr-1 animate-spin" />}
							{deleting ? "Deleting..." : "Delete"}
						</AlertDialogAction>
					)}
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
