import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import type { BranchInfo } from "@/types/git";

interface SettingsDialogProps {
	open: boolean;
	repoPath: string;
	onBaseBranchSaved: () => void;
	onClose: () => void;
}

export function SettingsDialog({
	open,
	repoPath,
	onBaseBranchSaved,
	onClose,
}: SettingsDialogProps) {
	const [branches, setBranches] = useState<BranchInfo[]>([]);
	const [selectedBase, setSelectedBase] = useState<string>("");
	const [initialBase, setInitialBase] = useState<string>("");
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (!open) return;
		setError(null);
		setSaving(false);

		Promise.all([
			invoke<BranchInfo[]>("list_branches", { repoPath }),
			invoke<string | null>("get_releash_base", { repoPath }),
		])
			.then(([branchList, currentBase]) => {
				setBranches(branchList.filter((b) => !b.is_remote));
				const base = currentBase ?? "";
				setSelectedBase(base);
				setInitialBase(base);
			})
			.catch((e) => {
				setError(String(e));
			});
	}, [open, repoPath]);

	const handleSave = useCallback(async () => {
		setSaving(true);
		setError(null);
		try {
			await invoke("set_releash_base", {
				repoPath,
				base: selectedBase || null,
			});
			onBaseBranchSaved();
			onClose();
		} catch (e) {
			setError(String(e));
		} finally {
			setSaving(false);
		}
	}, [selectedBase, repoPath, onBaseBranchSaved, onClose]);

	const isDirty = selectedBase !== initialBase;

	const labelClass = "text-xs font-medium text-muted-foreground";
	const selectClass =
		"w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary";

	return (
		<AlertDialog open={open} onOpenChange={(o) => !o && onClose()}>
			<AlertDialogContent className="max-w-md">
				<AlertDialogHeader>
					<AlertDialogTitle>Repository Settings</AlertDialogTitle>
				</AlertDialogHeader>

				<div className="grid gap-5 text-sm">
					<div className="flex flex-col gap-3">
						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-base-branch" className={labelClass}>
								Base branch for merge status detection
							</label>
							<select
								id="sd-base-branch"
								value={selectedBase}
								onChange={(e) => setSelectedBase(e.target.value)}
								className={selectClass}
							>
								<option value="">Auto (main/master)</option>
								{branches.map((b) => (
									<option key={b.name} value={b.name}>
										{b.name}
									</option>
								))}
							</select>
							{error && <p className="text-xs text-destructive">{error}</p>}
						</div>
					</div>
				</div>

				<AlertDialogFooter>
					<AlertDialogCancel>Close</AlertDialogCancel>
					<Button onClick={handleSave} disabled={!isDirty || saving}>
						{saving ? "..." : "Save"}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
