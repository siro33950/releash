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
import type { ClientTransportError } from "@/lib/clientSocket";
import { getErrorMessage } from "@/lib/errorMessage";
import type { WorktreeBranch } from "@/types/git";

interface DeleteWorktreeDialogProps {
	open: boolean;
	branch: WorktreeBranch | null;
	onConfirm: (
		branch: WorktreeBranch,
		force: boolean,
		onUncertain: (error: ClientTransportError) => void,
	) => Promise<void>;
	onCancel: () => void;
}

export function DeleteWorktreeDialog({
	open,
	branch,
	onConfirm,
	onCancel,
}: DeleteWorktreeDialogProps) {
	const [deleting, setDeleting] = useState(false);
	const [uncertain, setUncertain] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const deletingRef = useRef(false);

	const hasDirty = (branch?.dirty_count ?? 0) > 0;

	const handleDelete = useCallback(
		async (force: boolean) => {
			if (!branch || (!branch.worktree_path && !branch.is_merged)) return;
			setDeleting(true);
			setUncertain(false);
			deletingRef.current = true;
			setError(null);
			try {
				await onConfirm(branch, force, (cause) => {
					setUncertain(true);
					setError(cause.message);
				});
				setError(null);
			} catch (e) {
				setError(getErrorMessage(e));
			} finally {
				setDeleting(false);
				setUncertain(false);
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

	const isMergedWithWorktree = branch.is_merged && !!branch.worktree_path;

	const title = branch.is_merged ? "Delete Branch" : "Delete Workspace";
	const description = isMergedWithWorktree
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
					{error && (
						<p role="alert" className="text-destructive">
							{error}
						</p>
					)}
				</div>
				<AlertDialogFooter>
					<AlertDialogCancel
						onClick={onCancel}
						disabled={deleting && !uncertain}
					>
						Cancel
					</AlertDialogCancel>
					{hasDirty ? (
						<AlertDialogAction
							variant="destructive"
							onClick={() => handleDelete(true)}
							disabled={deleting}
						>
							{deleting && !uncertain && (
								<Loader2 className="size-3.5 mr-1 animate-spin" />
							)}
							{deleting && !uncertain ? "Deleting..." : "Force Delete"}
						</AlertDialogAction>
					) : (
						<AlertDialogAction
							variant="destructive"
							onClick={() => handleDelete(false)}
							disabled={deleting}
						>
							{deleting && !uncertain && (
								<Loader2 className="size-3.5 mr-1 animate-spin" />
							)}
							{deleting && !uncertain ? "Deleting..." : "Delete"}
						</AlertDialogAction>
					)}
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
