import { invoke } from "@tauri-apps/api/core";
import { Check, Copy, Loader2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
	AGENT_CONFIGS,
	type AgentType,
	type AppSettings,
	type DiffBase,
	type DiffMode,
	type Theme,
} from "@/types/settings";

const AGENT_TYPE_KEYS = Object.keys(AGENT_CONFIGS) as AgentType[];

export interface SettingsPanelProps {
	settings: AppSettings;
	onSave: (settings: AppSettings) => void;
}

export function SettingsPanel({ settings, onSave }: SettingsPanelProps) {
	const [draft, setDraft] = useState<AppSettings>(settings);

	// Hooks state
	const [hooksConfig, setHooksConfig] = useState<string>("");
	const [hooksLoading, setHooksLoading] = useState(false);
	const [hooksApplying, setHooksApplying] = useState(false);
	const [hooksStatus, setHooksStatus] = useState<
		"active" | "token_mismatch" | "not_configured"
	>("not_configured");
	const [hooksCopied, setHooksCopied] = useState(false);
	const [hooksError, setHooksError] = useState<string | null>(null);
	const [hooksSuccess, setHooksSuccess] = useState(false);

	useEffect(() => {
		setDraft(settings);
	}, [settings]);

	// Load hooks config when agent is claude
	useEffect(() => {
		if (draft.agent !== "claude") return;
		setHooksLoading(true);
		setHooksError(null);
		setHooksSuccess(false);

		Promise.all([
			invoke<string>("generate_hooks_config"),
			invoke<string>("get_hooks_status"),
		])
			.then(([json, status]) => {
				setHooksConfig(json);
				setHooksStatus(
					status as "active" | "token_mismatch" | "not_configured",
				);
			})
			.catch((e) => {
				setHooksError(String(e));
			})
			.finally(() => {
				setHooksLoading(false);
			});
	}, [draft.agent]);

	const handleApplyHooks = useCallback(async () => {
		setHooksApplying(true);
		setHooksError(null);
		try {
			await invoke("apply_hooks_config", { configJson: hooksConfig });
			setHooksStatus("active");
			setHooksSuccess(true);
		} catch (e) {
			setHooksError(String(e));
		} finally {
			setHooksApplying(false);
		}
	}, [hooksConfig]);

	const handleCopyHooks = useCallback(async () => {
		try {
			await navigator.clipboard.writeText(hooksConfig);
			setHooksCopied(true);
			setTimeout(() => setHooksCopied(false), 2000);
		} catch (e) {
			setHooksError(`Copy failed: ${String(e)}`);
		}
	}, [hooksConfig]);

	const handleSave = useCallback(() => {
		onSave(draft);
	}, [draft, onSave]);

	const isDirty = JSON.stringify(draft) !== JSON.stringify(settings);
	const showAutoApprove =
		draft.agent !== "none" &&
		draft.agent !== "cursor" &&
		draft.agent !== "custom";

	const labelClass = "text-xs font-medium text-muted-foreground";
	const selectClass =
		"w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary";
	const sectionHeader =
		"text-xs font-semibold uppercase tracking-wide text-muted-foreground mt-2 mb-1";

	return (
		<div className="h-full flex flex-col bg-sidebar">
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate">
					Settings
				</span>
			</div>

			<ScrollArea className="flex-1 min-h-0">
				<div className="px-3 py-3 flex flex-col gap-4">
					<div className="flex flex-col gap-1.5">
						<label htmlFor="theme-select" className={labelClass}>
							Theme
						</label>
						<select
							id="theme-select"
							value={draft.theme}
							onChange={(e) =>
								setDraft((d) => ({ ...d, theme: e.target.value as Theme }))
							}
							className={selectClass}
						>
							<option value="dark">Dark</option>
							<option value="light">Light</option>
						</select>
					</div>

					<div className="flex flex-col gap-1.5">
						<label htmlFor="diff-base-select" className={labelClass}>
							Default Base
						</label>
						<select
							id="diff-base-select"
							value={draft.defaultDiffBase}
							onChange={(e) =>
								setDraft((d) => ({
									...d,
									defaultDiffBase: e.target.value as DiffBase,
								}))
							}
							className={selectClass}
						>
							<option value="staged">Staged</option>
							<option value="HEAD">HEAD</option>
						</select>
					</div>

					<div className="flex flex-col gap-1.5">
						<label htmlFor="diff-mode-select" className={labelClass}>
							Default View
						</label>
						<select
							id="diff-mode-select"
							value={draft.defaultDiffMode}
							onChange={(e) =>
								setDraft((d) => ({
									...d,
									defaultDiffMode: e.target.value as DiffMode,
								}))
							}
							className={selectClass}
						>
							<option value="gutter">Gutter</option>
							<option value="inline">Inline</option>
							<option value="split">Split</option>
						</select>
					</div>

					<div className="flex flex-col gap-1.5">
						<label htmlFor="font-size-slider" className={labelClass}>
							Font Size: {draft.fontSize}px
						</label>
						<input
							id="font-size-slider"
							type="range"
							min={12}
							max={24}
							step={1}
							value={draft.fontSize}
							onChange={(e) =>
								setDraft((d) => ({ ...d, fontSize: Number(e.target.value) }))
							}
							className="w-full accent-primary"
						/>
						<div className="flex justify-between text-[10px] text-muted-foreground">
							<span>12px</span>
							<span>24px</span>
						</div>
					</div>

					<div className="flex flex-col gap-1.5">
						<label htmlFor="agent-select" className={labelClass}>
							Agent
						</label>
						<select
							id="agent-select"
							value={draft.agent}
							onChange={(e) =>
								setDraft((d) => ({ ...d, agent: e.target.value as AgentType }))
							}
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
								onChange={(e) =>
									setDraft((d) => ({
										...d,
										agentAutoApprove: e.target.checked,
									}))
								}
								className="accent-primary"
							/>
							<span className={labelClass}>Auto-approve</span>
						</label>
					)}

					<label className="flex items-center gap-2 cursor-pointer">
						<input
							type="checkbox"
							checked={draft.autoUpdate}
							onChange={(e) =>
								setDraft((d) => ({
									...d,
									autoUpdate: e.target.checked,
								}))
							}
							className="accent-primary"
						/>
						<span className="text-xs font-medium text-muted-foreground">
							Auto-update
						</span>
					</label>

					{draft.agent === "custom" && (
						<div className="flex flex-col gap-1.5">
							<label htmlFor="terminal-startup-cmd" className={labelClass}>
								Startup Command
							</label>
							<textarea
								id="terminal-startup-cmd"
								value={draft.terminalStartupCommand}
								onChange={(e) =>
									setDraft((d) => ({
										...d,
										terminalStartupCommand: e.target.value,
									}))
								}
								placeholder="e.g. nvm use 18 && clear"
								rows={3}
								className="w-full bg-muted border border-border rounded px-2 py-1.5 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary resize-none"
							/>
							<p className="text-[10px] text-muted-foreground">
								Optional: pre-launch setup command.
							</p>
						</div>
					)}

					<Button
						size="sm"
						onClick={handleSave}
						disabled={!isDirty}
						className="w-full"
					>
						Save
					</Button>

					{/* Claude Code Hooks */}
					{draft.agent === "claude" && (
						<div className="flex flex-col gap-2">
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
											{hooksStatus === "active" && (
												<span className="text-green-500">Enabled</span>
											)}
											{hooksStatus === "token_mismatch" && (
												<span className="text-yellow-500">
													Token mismatch — 再設定が必要です
												</span>
											)}
											{hooksStatus === "not_configured" && (
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
											設定を適用しました。Claude
											Codeを再起動すると反映されます。
										</p>
									)}

									<div className="flex justify-end">
										<Button
											size="sm"
											variant={hooksStatus === "active" ? "ghost" : "default"}
											onClick={handleApplyHooks}
											disabled={hooksApplying || !hooksConfig}
										>
											{hooksApplying ? (
												<Loader2 className="size-3.5 mr-1 animate-spin" />
											) : null}
											{hooksStatus === "active" ? "再設定" : "設定を適用"}
										</Button>
									</div>
								</>
							)}
						</div>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}
