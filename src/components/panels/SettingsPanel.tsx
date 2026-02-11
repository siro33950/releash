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
	onOpenRepoSettings?: () => void;
}

export function SettingsPanel({
	settings,
	onSave,
	onOpenRepoSettings,
}: SettingsPanelProps) {
	const [draft, setDraft] = useState<AppSettings>(settings);

	useEffect(() => {
		setDraft(settings);
	}, [settings]);

	const handleSave = useCallback(() => {
		onSave(draft);
	}, [draft, onSave]);

	const isDirty = JSON.stringify(draft) !== JSON.stringify(settings);
	const showAutoApprove =
		draft.agent !== "none" &&
		draft.agent !== "cursor" &&
		draft.agent !== "custom";

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
						<label
							htmlFor="theme-select"
							className="text-xs font-medium text-muted-foreground"
						>
							Theme
						</label>
						<select
							id="theme-select"
							value={draft.theme}
							onChange={(e) =>
								setDraft((d) => ({ ...d, theme: e.target.value as Theme }))
							}
							className="w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
						>
							<option value="dark">Dark</option>
							<option value="light">Light</option>
						</select>
					</div>

					<div className="flex flex-col gap-1.5">
						<label
							htmlFor="diff-base-select"
							className="text-xs font-medium text-muted-foreground"
						>
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
							className="w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
						>
							<option value="staged">Staged</option>
							<option value="HEAD">HEAD</option>
						</select>
					</div>

					<div className="flex flex-col gap-1.5">
						<label
							htmlFor="diff-mode-select"
							className="text-xs font-medium text-muted-foreground"
						>
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
							className="w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
						>
							<option value="gutter">Gutter</option>
							<option value="inline">Inline</option>
							<option value="split">Split</option>
						</select>
					</div>

					<div className="flex flex-col gap-1.5">
						<label
							htmlFor="font-size-slider"
							className="text-xs font-medium text-muted-foreground"
						>
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
						<label
							htmlFor="agent-select"
							className="text-xs font-medium text-muted-foreground"
						>
							Agent
						</label>
						<select
							id="agent-select"
							value={draft.agent}
							onChange={(e) =>
								setDraft((d) => ({ ...d, agent: e.target.value as AgentType }))
							}
							className="w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
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
							<span className="text-xs font-medium text-muted-foreground">
								Auto-approve
							</span>
						</label>
					)}

					{draft.agent === "custom" && (
						<div className="flex flex-col gap-1.5">
							<label
								htmlFor="terminal-startup-cmd"
								className="text-xs font-medium text-muted-foreground"
							>
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

					{onOpenRepoSettings && (
						<Button
							size="sm"
							variant="outline"
							onClick={onOpenRepoSettings}
							className="w-full"
						>
							Repo Settings...
						</Button>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}
