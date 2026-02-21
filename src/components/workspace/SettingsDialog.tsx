import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useReducer } from "react";
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

interface SettingsDialogState {
	branches: BranchInfo[];
	selectedBase: string;
	initialBase: string;
	saving: boolean;
	error: string | null;
}

type SettingsDialogAction =
	| { type: "LOAD_SUCCESS"; branches: BranchInfo[]; base: string }
	| { type: "LOAD_ERROR"; error: string }
	| { type: "RESET" }
	| { type: "SELECT_BASE"; base: string }
	| { type: "SAVE_START" }
	| { type: "SAVE_ERROR"; error: string }
	| { type: "SAVE_END" };

const initialState: SettingsDialogState = {
	branches: [],
	selectedBase: "",
	initialBase: "",
	saving: false,
	error: null,
};

function settingsDialogReducer(
	state: SettingsDialogState,
	action: SettingsDialogAction,
): SettingsDialogState {
	switch (action.type) {
		case "RESET":
			return { ...initialState };
		case "LOAD_SUCCESS":
			return {
				...state,
				branches: action.branches,
				selectedBase: action.base,
				initialBase: action.base,
				error: null,
			};
		case "LOAD_ERROR":
			return { ...state, error: action.error };
		case "SELECT_BASE":
			return { ...state, selectedBase: action.base };
		case "SAVE_START":
			return { ...state, saving: true, error: null };
		case "SAVE_ERROR":
			return { ...state, saving: false, error: action.error };
		case "SAVE_END":
			return { ...state, saving: false };
	}
}

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
	const [state, dispatch] = useReducer(settingsDialogReducer, initialState);
	const { branches, selectedBase, initialBase, saving, error } = state;

	useEffect(() => {
		if (!open) return;
		dispatch({ type: "RESET" });

		Promise.all([
			invoke<BranchInfo[]>("list_branches", { repoPath }),
			invoke<string | null>("get_releash_base", { repoPath }),
		])
			.then(([branchList, currentBase]) => {
				dispatch({
					type: "LOAD_SUCCESS",
					branches: branchList.filter((b) => !b.is_remote),
					base: currentBase ?? "",
				});
			})
			.catch((e) => {
				dispatch({ type: "LOAD_ERROR", error: String(e) });
			});
	}, [open, repoPath]);

	const handleSave = useCallback(async () => {
		dispatch({ type: "SAVE_START" });
		try {
			await invoke("set_releash_base", {
				repoPath,
				base: selectedBase || null,
			});
			dispatch({ type: "SAVE_END" });
			onBaseBranchSaved();
			onClose();
		} catch (e) {
			dispatch({ type: "SAVE_ERROR", error: String(e) });
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
								onChange={(e) =>
									dispatch({ type: "SELECT_BASE", base: e.target.value })
								}
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
