import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import type { BranchInfo } from "@/types/git";

interface BaseBranchDialogProps {
	open: boolean;
	repoPath: string;
	onSaved: () => void;
	onCancel: () => void;
}

export function BaseBranchDialog({
	open,
	repoPath,
	onSaved,
	onCancel,
}: BaseBranchDialogProps) {
	const [branches, setBranches] = useState<BranchInfo[]>([]);
	const [selected, setSelected] = useState<string>("");
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (!open) return;
		setError(null);
		setSaving(false);

		Promise.all([
			invoke<BranchInfo[]>("list_branches", { filePath: repoPath }),
			invoke<string | null>("get_releash_base", { repoPath }),
		]).then(([branchList, currentBase]) => {
			setBranches(branchList.filter((b) => !b.is_remote));
			setSelected(currentBase ?? "");
		});
	}, [open, repoPath]);

	const handleSave = useCallback(async () => {
		setSaving(true);
		setError(null);
		try {
			await invoke("set_releash_base", {
				repoPath,
				base: selected || null,
			});
			onSaved();
		} catch (e) {
			setError(String(e));
		} finally {
			setSaving(false);
		}
	}, [repoPath, selected, onSaved]);

	return (
		<AlertDialog open={open} onOpenChange={(o) => !o && onCancel()}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>Base Branch</AlertDialogTitle>
					<AlertDialogDescription>
						Select the base branch for merge status detection.
					</AlertDialogDescription>
				</AlertDialogHeader>
				<div className="grid gap-3 text-sm">
					<select
						value={selected}
						onChange={(e) => setSelected(e.target.value)}
						className="h-8 rounded-md border border-input bg-background px-2 text-sm"
					>
						<option value="">Auto (main/master)</option>
						{branches.map((b) => (
							<option key={b.name} value={b.name}>
								{b.name}
							</option>
						))}
					</select>
					{error && <p className="text-sm text-destructive">{error}</p>}
				</div>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={onCancel} disabled={saving}>
						Cancel
					</AlertDialogCancel>
					<Button onClick={handleSave} disabled={saving}>
						{saving ? "Saving..." : "Save"}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
