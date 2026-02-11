import { invoke } from "@tauri-apps/api/core";
import { Check, Copy, Loader2 } from "lucide-react";
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
import {
	AGENT_CONFIGS,
	type AgentType,
	type AppSettings,
	type DiffBase,
	type DiffMode,
	type Theme,
} from "@/types/settings";

interface SettingsDialogProps {
	open: boolean;
	repoPath: string;
	settings: AppSettings;
	onSave: (settings: AppSettings) => void;
	onBaseBranchSaved: () => void;
	onClose: () => void;
}

const AGENT_TYPE_KEYS = Object.keys(AGENT_CONFIGS) as AgentType[];

export function SettingsDialog({
	open,
	repoPath,
	settings,
	onSave,
	onBaseBranchSaved,
	onClose,
}: SettingsDialogProps) {
	const [draft, setDraft] = useState<AppSettings>(settings);
	const [branches, setBranches] = useState<BranchInfo[]>([]);
	const [selectedBase, setSelectedBase] = useState<string>("");
	const [initialBase, setInitialBase] = useState<string>("");
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	// Hooks state
	const [hooksConfig, setHooksConfig] = useState<string>("");
	const [hooksLoading, setHooksLoading] = useState(false);
	const [hooksApplying, setHooksApplying] = useState(false);
	const [hooksEnabled, setHooksEnabled] = useState(false);
	const [hooksCopied, setHooksCopied] = useState(false);
	const [hooksError, setHooksError] = useState<string | null>(null);
	const [hooksSuccess, setHooksSuccess] = useState(false);

	// Reset draft when dialog opens
	useEffect(() => {
		if (open) {
			setDraft(settings);
		}
	}, [open, settings]);

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

	// Load hooks config when dialog opens and agent is claude
	useEffect(() => {
		if (!open || draft.agent !== "claude") return;
		setHooksLoading(true);
		setHooksError(null);
		setHooksSuccess(false);

		Promise.all([
			invoke<string>("generate_hooks_config"),
			invoke<boolean>("get_hooks_status"),
		])
			.then(([json, status]) => {
				setHooksConfig(json);
				setHooksEnabled(status);
			})
			.catch((e) => {
				setHooksError(String(e));
			})
			.finally(() => {
				setHooksLoading(false);
			});
	}, [open, draft.agent]);

	const handleApplyHooks = useCallback(async () => {
		setHooksApplying(true);
		setHooksError(null);
		try {
			await invoke("apply_hooks_config", { configJson: hooksConfig });
			setHooksEnabled(true);
			setHooksSuccess(true);
		} catch (e) {
			setHooksError(String(e));
		} finally {
			setHooksApplying(false);
		}
	}, [hooksConfig]);

	const handleCopyHooks = useCallback(async () => {
		await navigator.clipboard.writeText(hooksConfig);
		setHooksCopied(true);
		setTimeout(() => setHooksCopied(false), 2000);
	}, [hooksConfig]);

	const handleSave = useCallback(async () => {
		setSaving(true);
		setError(null);
		try {
			onSave(draft);
			if (selectedBase !== initialBase) {
				await invoke("set_releash_base", {
					repoPath,
					base: selectedBase || null,
				});
				onBaseBranchSaved();
			}
			onClose();
		} catch (e) {
			setError(String(e));
		} finally {
			setSaving(false);
		}
	}, [draft, selectedBase, initialBase, repoPath, onSave, onBaseBranchSaved, onClose]);

	const settingsDirty = JSON.stringify(draft) !== JSON.stringify(settings);
	const baseDirty = selectedBase !== initialBase;
	const isDirty = settingsDirty || baseDirty;

	const showAutoApprove = draft.agent !== "none" && draft.agent !== "cursor" && draft.agent !== "custom";

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
								value={draft.theme}
								onChange={(e) => setDraft((d) => ({ ...d, theme: e.target.value as Theme }))}
								className={selectClass}
							>
								<option value="dark">Dark</option>
								<option value="light">Light</option>
							</select>
						</div>

						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-font-size" className={labelClass}>
								Font Size: {draft.fontSize}px
							</label>
							<input
								id="sd-font-size"
								type="range"
								min={12}
								max={24}
								step={1}
								value={draft.fontSize}
								onChange={(e) => setDraft((d) => ({ ...d, fontSize: Number(e.target.value) }))}
								className="w-full accent-primary"
							/>
						</div>

						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-diff-base" className={labelClass}>
								Default Base
							</label>
							<select
								id="sd-diff-base"
								value={draft.defaultDiffBase}
								onChange={(e) => setDraft((d) => ({ ...d, defaultDiffBase: e.target.value as DiffBase }))}
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
								value={draft.defaultDiffMode}
								onChange={(e) => setDraft((d) => ({ ...d, defaultDiffMode: e.target.value as DiffMode }))}
								className={selectClass}
							>
								<option value="gutter">Gutter</option>
								<option value="inline">Inline</option>
								<option value="split">Split</option>
							</select>
						</div>
					</div>

					{/* Agent */}
					<div className="flex flex-col gap-3">
						<h3 className={sectionHeader}>Agent</h3>

						<div className="flex flex-col gap-1.5">
							<label htmlFor="sd-agent" className={labelClass}>
								Agent
							</label>
							<select
								id="sd-agent"
								value={draft.agent}
								onChange={(e) => setDraft((d) => ({ ...d, agent: e.target.value as AgentType }))}
								className={selectClass}
							>
								{AGENT_TYPE_KEYS.map((key) => (
									<option key={key} value={key}>
										{AGENT_CONFIGS[key].label}
									</option>
								))}
							</select>
						</div>

						{showAutoApprove && (
							<label className="flex items-center gap-2 cursor-pointer">
								<input
									type="checkbox"
									checked={draft.agentAutoApprove}
									onChange={(e) => setDraft((d) => ({ ...d, agentAutoApprove: e.target.checked }))}
									className="accent-primary"
								/>
								<span className={labelClass}>Auto-approve</span>
							</label>
						)}

						{draft.agent === "custom" && (
							<div className="flex flex-col gap-1.5">
								<label htmlFor="sd-startup-cmd" className={labelClass}>
									Startup Command
								</label>
								<textarea
									id="sd-startup-cmd"
									value={draft.terminalStartupCommand}
									onChange={(e) => setDraft((d) => ({ ...d, terminalStartupCommand: e.target.value }))}
									placeholder="e.g. nvm use 18 && clear"
									rows={2}
									className="w-full bg-muted border border-border rounded px-2 py-1.5 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary resize-none"
								/>
								<p className="text-[10px] text-muted-foreground">
									Optional: pre-launch setup command.
								</p>
							</div>
						)}
					</div>

					{/* Claude Code Hooks */}
					{draft.agent === "claude" && (
						<div className="flex flex-col gap-3">
							<h3 className={sectionHeader}>Claude Code Hooks</h3>

							{hooksLoading ? (
								<div className="flex items-center justify-center py-4">
									<Loader2 className="size-4 animate-spin text-muted-foreground" />
								</div>
							) : (
								<>
									<div className="flex items-center gap-2">
										<span className="text-xs font-medium">
											Status:{" "}
											{hooksEnabled ? (
												<span className="text-green-500">Enabled</span>
											) : (
												<span className="text-muted-foreground">
													Not configured
												</span>
											)}
										</span>
									</div>

									<div className="relative">
										<pre className="max-h-40 overflow-auto rounded border border-border bg-muted/50 p-2 text-[10px] font-mono whitespace-pre-wrap break-all">
											{hooksConfig}
										</pre>
										<button
											type="button"
											className="absolute top-1.5 right-1.5 p-1 rounded bg-background/80 border border-border hover:bg-muted transition-colors"
											onClick={handleCopyHooks}
										>
											{hooksCopied ? (
												<Check className="size-3 text-green-500" />
											) : (
												<Copy className="size-3 text-muted-foreground" />
											)}
										</button>
									</div>

									{hooksError && (
										<p className="text-xs text-red-500">{hooksError}</p>
									)}

									{hooksSuccess && (
										<p className="text-xs text-green-500">
											設定を適用しました。Claude Codeを再起動すると反映されます。
										</p>
									)}

									<div className="flex justify-end">
										<Button
											size="sm"
											variant={hooksEnabled ? "ghost" : "default"}
											onClick={handleApplyHooks}
											disabled={hooksApplying || !hooksConfig}
										>
											{hooksApplying ? (
												<Loader2 className="size-3.5 mr-1 animate-spin" />
											) : null}
											{hooksEnabled ? "再設定" : "設定を適用"}
										</Button>
									</div>
								</>
							)}
						</div>
					)}

					{/* Base Branch */}
					<div className="flex flex-col gap-3">
						<h3 className={sectionHeader}>Base Branch</h3>
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
					<AlertDialogCancel onClick={onClose}>Close</AlertDialogCancel>
					<Button onClick={handleSave} disabled={!isDirty || saving}>
						{saving ? "..." : "Save"}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
