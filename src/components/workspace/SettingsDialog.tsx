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
import type { AppSettings, DiffBase, DiffMode, Theme } from "@/types/settings";

interface SettingsDialogProps {
	open: boolean;
	repoPath: string;
	settings: AppSettings;
	onThemeChange: (theme: Theme) => void;
	onFontSizeChange: (size: number) => void;
	onDiffBaseChange: (base: DiffBase) => void;
	onDiffModeChange: (mode: DiffMode) => void;
	onTerminalStartupCommandChange: (command: string) => void;
	onBaseBranchSaved: () => void;
	onClose: () => void;
}

export function SettingsDialog({
	open,
	repoPath,
	settings,
	onThemeChange,
	onFontSizeChange,
	onDiffBaseChange,
	onDiffModeChange,
	onTerminalStartupCommandChange,
	onBaseBranchSaved,
	onClose,
}: SettingsDialogProps) {
	const [branches, setBranches] = useState<BranchInfo[]>([]);
	const [selectedBase, setSelectedBase] = useState<string>("");
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
				setSelectedBase(currentBase ?? "");
			})
			.catch((e) => {
				setError(String(e));
			});
	}, [open, repoPath]);

	const handleSaveBaseBranch = useCallback(async () => {
		setSaving(true);
		setError(null);
		try {
			await invoke("set_releash_base", {
				repoPath,
				base: selectedBase || null,
			});
			onBaseBranchSaved();
		} catch (e) {
			setError(String(e));
		} finally {
			setSaving(false);
		}
	}, [repoPath, selectedBase, onBaseBranchSaved]);

	const sectionHeader =
		"text-xs font-semibold uppercase tracking-wide text-muted-foreground mb-2";
	const labelClass = "text-xs font-medium text-muted-foreground";
	const selectClass =
		"w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary";

	return (
		<AlertDialog open={open} onOpenChange={(o) => !o && onClose()}>
			<AlertDialogContent className="max-w-md">
				<AlertDialogHeader>
					<AlertDialogTitle>Settings</AlertDialogTitle>
				</AlertDialogHeader>

				<div className="grid gap-5 text-sm max-h-[60vh] overflow-y-auto pr-1">
					{/* App Settings */}
					<div className="flex flex-col gap-3">
						<h3 className={sectionHeader}>App Settings</h3>

						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-theme" className={labelClass}>
								Theme
							</label>
							<select
								id="sd-theme"
								value={settings.theme}
								onChange={(e) => onThemeChange(e.target.value as Theme)}
								className={selectClass}
							>
								<option value="dark">Dark</option>
								<option value="light">Light</option>
							</select>
						</div>

						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-font-size" className={labelClass}>
								Font Size: {settings.fontSize}px
							</label>
							<input
								id="sd-font-size"
								type="range"
								min={12}
								max={24}
								step={1}
								value={settings.fontSize}
								onChange={(e) => onFontSizeChange(Number(e.target.value))}
								className="w-full accent-primary"
							/>
						</div>

						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-diff-base" className={labelClass}>
								Default Base
							</label>
							<select
								id="sd-diff-base"
								value={settings.defaultDiffBase}
								onChange={(e) => onDiffBaseChange(e.target.value as DiffBase)}
								className={selectClass}
							>
								<option value="staged">Staged</option>
								<option value="HEAD">HEAD</option>
							</select>
						</div>

						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-diff-mode" className={labelClass}>
								Default View
							</label>
							<select
								id="sd-diff-mode"
								value={settings.defaultDiffMode}
								onChange={(e) => onDiffModeChange(e.target.value as DiffMode)}
								className={selectClass}
							>
								<option value="gutter">Gutter</option>
								<option value="inline">Inline</option>
								<option value="split">Split</option>
							</select>
						</div>
					</div>

					{/* Terminal */}
					<div className="flex flex-col gap-3">
						<h3 className={sectionHeader}>Terminal</h3>
						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-startup-cmd" className={labelClass}>
								Startup Command
							</label>
							<textarea
								id="sd-startup-cmd"
								value={settings.terminalStartupCommand}
								onChange={(e) => onTerminalStartupCommandChange(e.target.value)}
								placeholder="e.g. nvm use 18 && clear"
								rows={3}
								className="w-full bg-muted border border-border rounded px-2 py-1.5 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary resize-none"
							/>
							<p className="text-[10px] text-muted-foreground">
								Command to run when a new terminal is opened.
							</p>
						</div>
					</div>

					{/* Base Branch */}
					<div className="flex flex-col gap-3">
						<h3 className={sectionHeader}>Base Branch</h3>
						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-base-branch" className={labelClass}>
								Base branch for merge status detection
							</label>
							<div className="flex gap-2">
								<select
									id="sd-base-branch"
									value={selectedBase}
									onChange={(e) => setSelectedBase(e.target.value)}
									className={`flex-1 ${selectClass}`}
								>
									<option value="">Auto (main/master)</option>
									{branches.map((b) => (
										<option key={b.name} value={b.name}>
											{b.name}
										</option>
									))}
								</select>
								<Button
									size="sm"
									onClick={handleSaveBaseBranch}
									disabled={saving}
								>
									{saving ? "..." : "Save"}
								</Button>
							</div>
							{error && <p className="text-xs text-destructive">{error}</p>}
						</div>
					</div>
				</div>

				<AlertDialogFooter>
					<AlertDialogCancel onClick={onClose}>Close</AlertDialogCancel>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
