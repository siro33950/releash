import { invoke } from "@tauri-apps/api/core";
import {
	Bell,
	Bot,
	Check,
	Code,
	Copy,
	Globe,
	Loader2,
	Palette,
	Shield,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useRemoteConfig } from "@/hooks/useRemoteConfig";
import { useWebhookConfig } from "@/hooks/useWebhookConfig";
import { trackEvent } from "@/lib/telemetry";
import { cn } from "@/lib/utils";
import {
	AGENT_CONFIGS,
	type AgentType,
	type AppSettings,
	type DiffBase,
	type DiffMode,
	type Theme,
} from "@/types/settings";
import {
	type DesktopNotifyMode,
	INACTIVE_TIMEOUT_OPTIONS,
} from "@/types/webhook";

const AGENT_TYPE_KEYS = Object.keys(AGENT_CONFIGS) as AgentType[];

type SettingsSection =
	| "appearance"
	| "editor"
	| "agent"
	| "remote"
	| "notifications"
	| "privacy";

const SETTINGS_SECTIONS: {
	id: SettingsSection;
	label: string;
	icon: React.ComponentType<{ className?: string }>;
}[] = [
	{ id: "appearance", label: "Appearance", icon: Palette },
	{ id: "editor", label: "Editor", icon: Code },
	{ id: "agent", label: "Agent", icon: Bot },
	{ id: "remote", label: "Remote", icon: Globe },
	{ id: "notifications", label: "Notifications", icon: Bell },
	{ id: "privacy", label: "Privacy & Updates", icon: Shield },
];

const labelClass = "text-xs font-medium text-muted-foreground";
const selectClass =
	"w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary";

function AppearanceSection({
	draft,
	updateDraft,
}: {
	draft: AppSettings;
	updateDraft: (updater: (d: AppSettings) => AppSettings) => void;
}) {
	return (
		<div className="flex flex-col gap-4">
			<div className="flex flex-col gap-1.5">
				<label htmlFor="theme-select" className={labelClass}>
					Theme
				</label>
				<select
					id="theme-select"
					value={draft.theme}
					onChange={(e) =>
						updateDraft((d) => ({ ...d, theme: e.target.value as Theme }))
					}
					className={selectClass}
				>
					<option value="dark">Dark</option>
					<option value="light">Light</option>
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
						updateDraft((d) => ({ ...d, fontSize: Number(e.target.value) }))
					}
					className="w-full accent-primary"
				/>
				<div className="flex justify-between text-[10px] text-muted-foreground">
					<span>12px</span>
					<span>24px</span>
				</div>
			</div>
		</div>
	);
}

function EditorSection({
	draft,
	updateDraft,
}: {
	draft: AppSettings;
	updateDraft: (updater: (d: AppSettings) => AppSettings) => void;
}) {
	return (
		<div className="flex flex-col gap-4">
			<div className="flex flex-col gap-1.5">
				<label htmlFor="diff-base-select" className={labelClass}>
					Default Base
				</label>
				<select
					id="diff-base-select"
					value={draft.defaultDiffBase}
					onChange={(e) =>
						updateDraft((d) => ({
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
						updateDraft((d) => ({
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
		</div>
	);
}

function AgentSection({
	draft,
	updateDraft,
	hooksConfig,
	hooksLoading,
	hooksApplying,
	hooksStatus,
	hooksCopied,
	hooksError,
	hooksSuccess,
	onApplyHooks,
	onCopyHooks,
}: {
	draft: AppSettings;
	updateDraft: (updater: (d: AppSettings) => AppSettings) => void;
	hooksConfig: string;
	hooksLoading: boolean;
	hooksApplying: boolean;
	hooksStatus: "active" | "token_mismatch" | "not_configured";
	hooksCopied: boolean;
	hooksError: string | null;
	hooksSuccess: boolean;
	onApplyHooks: () => void;
	onCopyHooks: () => void;
}) {
	const showAutoApprove =
		draft.agent !== "none" &&
		draft.agent !== "cursor" &&
		draft.agent !== "custom";

	return (
		<div className="flex flex-col gap-4">
			<div className="flex flex-col gap-1.5">
				<label htmlFor="agent-select" className={labelClass}>
					Agent
				</label>
				<select
					id="agent-select"
					value={draft.agent}
					onChange={(e) =>
						updateDraft((d) => ({
							...d,
							agent: e.target.value as AgentType,
						}))
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
							updateDraft((d) => ({
								...d,
								agentAutoApprove: e.target.checked,
							}))
						}
						className="accent-primary"
					/>
					<span className={labelClass}>Auto-approve</span>
				</label>
			)}

			{draft.agent === "custom" && (
				<div className="flex flex-col gap-1.5">
					<label htmlFor="terminal-startup-cmd" className={labelClass}>
						Startup Command
					</label>
					<textarea
						id="terminal-startup-cmd"
						value={draft.terminalStartupCommand}
						onChange={(e) =>
							updateDraft((d) => ({
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

			{draft.agent === "claude" && (
				<div className="flex flex-col gap-2">
					<h4 className="text-xs font-semibold text-muted-foreground">
						Claude Code Hooks
					</h4>

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
										<span className="text-success">Enabled</span>
									)}
									{hooksStatus === "token_mismatch" && (
										<span className="text-warning">
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
									onClick={onCopyHooks}
								>
									{hooksCopied ? (
										<Check className="size-3 text-success" />
									) : (
										<Copy className="size-3 text-muted-foreground" />
									)}
								</button>
							</div>

							{hooksError && (
								<p className="text-xs text-destructive">{hooksError}</p>
							)}

							{hooksSuccess && (
								<p className="text-xs text-success">
									設定を適用しました。Claude Codeを再起動すると反映されます。
								</p>
							)}

							<div className="flex justify-end">
								<Button
									size="sm"
									variant={hooksStatus === "active" ? "ghost" : "default"}
									onClick={onApplyHooks}
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
	);
}

function RemoteSection({
	remote,
}: {
	remote: ReturnType<typeof useRemoteConfig>;
}) {
	return (
		<div className="flex flex-col gap-2">
			{remote.loading ? (
				<div className="flex items-center justify-center py-4">
					<Loader2 className="size-4 animate-spin text-muted-foreground" />
				</div>
			) : (
				<>
					<label className="flex items-center gap-2 cursor-pointer">
						<input
							type="checkbox"
							checked={remote.draft.auto_start}
							onChange={(e) =>
								remote.setDraft((d) => ({
									...d,
									auto_start: e.target.checked,
									auto_start_on_lan: e.target.checked
										? d.auto_start_on_lan
										: false,
								}))
							}
							className="accent-primary"
						/>
						<span className={labelClass}>Auto-start remote server</span>
					</label>

					<label
						className={`flex items-center gap-2 ml-4 ${remote.draft.auto_start ? "cursor-pointer" : "cursor-not-allowed opacity-50"}`}
					>
						<input
							type="checkbox"
							checked={remote.draft.auto_start_on_lan}
							disabled={!remote.draft.auto_start}
							onChange={(e) =>
								remote.setDraft((d) => ({
									...d,
									auto_start_on_lan: e.target.checked,
								}))
							}
							className="accent-primary"
						/>
						<span className={labelClass}>Allow auto-start on LAN</span>
					</label>

					<p className="text-[10px] text-muted-foreground">
						VPN接続時は常に自動起動します。LAN接続時の自動起動は上記で制御できます。
					</p>

					{remote.error && (
						<p className="text-xs text-destructive">{remote.error}</p>
					)}
				</>
			)}
		</div>
	);
}

function NotificationsSection({
	webhook,
}: {
	webhook: ReturnType<typeof useWebhookConfig>;
}) {
	const webhookUrlValue = webhook.draft.webhook_url;
	const detectedWebhookType =
		webhookUrlValue.includes("discord.com/api/webhooks/") ||
		webhookUrlValue.includes("discordapp.com/api/webhooks/")
			? "Discord"
			: webhookUrlValue.includes("hooks.slack.com/")
				? "Slack"
				: webhookUrlValue
					? "Generic (Slack format)"
					: null;

	return (
		<div className="flex flex-col gap-2">
			{webhook.loading ? (
				<div className="flex items-center justify-center py-4">
					<Loader2 className="size-4 animate-spin text-muted-foreground" />
				</div>
			) : (
				<>
					<div className="flex flex-col gap-1.5">
						<label htmlFor="webhook-url" className={labelClass}>
							Webhook URL
						</label>
						<Input
							id="webhook-url"
							type="url"
							variant="panel"
							size="sm"
							value={webhook.draft.webhook_url}
							onChange={(e) =>
								webhook.setDraft((d) => ({
									...d,
									webhook_url: e.target.value,
								}))
							}
							placeholder="https://hooks.slack.com/... or https://discord.com/api/webhooks/..."
						/>
						{detectedWebhookType && (
							<p className="text-[10px] text-muted-foreground">
								Detected: {detectedWebhookType}
							</p>
						)}
					</div>

					<div className="flex flex-col gap-1.5">
						<span className={labelClass}>Notify on</span>
						<div className="flex flex-wrap gap-x-3 gap-y-1">
							{(
								[
									["on_running", "Running"],
									["on_done", "Done"],
									["on_error", "Error"],
									["on_waiting", "Waiting"],
								] as const
							).map(([key, label]) => (
								<label
									key={key}
									className="flex items-center gap-1 cursor-pointer"
								>
									<input
										type="checkbox"
										checked={webhook.draft[key]}
										onChange={(e) =>
											webhook.setDraft((d) => ({
												...d,
												[key]: e.target.checked,
											}))
										}
										className="accent-primary"
									/>
									<span className="text-xs">{label}</span>
								</label>
							))}
						</div>
					</div>

					<div className="flex flex-col gap-1.5">
						<span className={labelClass}>Send notifications</span>
						<label className="flex items-center gap-2 cursor-pointer">
							<input
								type="radio"
								name="desktop-mode"
								value="always"
								checked={webhook.draft.desktop_mode === "always"}
								onChange={() =>
									webhook.setDraft((d) => ({
										...d,
										desktop_mode: "always" as DesktopNotifyMode,
									}))
								}
								className="accent-primary"
							/>
							<span className="text-xs">Always</span>
						</label>
						<label className="flex items-center gap-2 cursor-pointer">
							<input
								type="radio"
								name="desktop-mode"
								value="when_inactive"
								checked={webhook.draft.desktop_mode === "when_inactive"}
								onChange={() =>
									webhook.setDraft((d) => ({
										...d,
										desktop_mode: "when_inactive" as DesktopNotifyMode,
									}))
								}
								className="accent-primary"
							/>
							<span className="text-xs">When inactive for</span>
							{webhook.draft.desktop_mode === "when_inactive" && (
								<select
									value={webhook.draft.inactive_timeout_minutes}
									onChange={(e) =>
										webhook.setDraft((d) => ({
											...d,
											inactive_timeout_minutes: Number(e.target.value),
										}))
									}
									className="bg-muted border border-border rounded px-1.5 py-0.5 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
								>
									{INACTIVE_TIMEOUT_OPTIONS.map((opt) => (
										<option key={opt.value} value={opt.value}>
											{opt.label}
										</option>
									))}
								</select>
							)}
						</label>
					</div>

					{webhook.error && (
						<p className="text-xs text-destructive">{webhook.error}</p>
					)}
				</>
			)}
		</div>
	);
}

function PrivacySection({
	draft,
	updateDraft,
}: {
	draft: AppSettings;
	updateDraft: (updater: (d: AppSettings) => AppSettings) => void;
}) {
	return (
		<div className="flex flex-col gap-4">
			<label className="flex items-center gap-2 cursor-pointer">
				<input
					type="checkbox"
					checked={draft.autoUpdate}
					onChange={(e) =>
						updateDraft((d) => ({
							...d,
							autoUpdate: e.target.checked,
						}))
					}
					className="accent-primary"
				/>
				<span className={labelClass}>Auto-update</span>
			</label>

			<label className="flex items-center gap-2 cursor-pointer">
				<input
					type="checkbox"
					checked={draft.telemetryEnabled}
					onChange={(e) =>
						updateDraft((d) => ({
							...d,
							telemetryEnabled: e.target.checked,
						}))
					}
					className="accent-primary"
				/>
				<span className={labelClass}>Send anonymous usage data</span>
			</label>

			<label className="flex items-center gap-2 cursor-pointer">
				<input
					type="checkbox"
					checked={draft.enableCrashReporting}
					onChange={(e) =>
						updateDraft((d) => ({
							...d,
							enableCrashReporting: e.target.checked,
						}))
					}
					className="accent-primary"
				/>
				<span className={labelClass}>Send crash reports</span>
			</label>
			<p className="text-[10px] text-muted-foreground -mt-2">
				Help improve Releash by sending anonymous crash reports.
			</p>
		</div>
	);
}

export interface SettingsModalProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	settings: AppSettings;
	onSave: (settings: AppSettings) => void;
}

export function SettingsModal({
	open,
	onOpenChange,
	settings,
	onSave,
}: SettingsModalProps) {
	const [activeSection, setActiveSection] =
		useState<SettingsSection>("appearance");
	const [draft, setDraft] = useState<AppSettings>(settings);
	const [appDirty, setAppDirty] = useState(false);
	const [saving, setSaving] = useState(false);
	const webhook = useWebhookConfig();
	const remote = useRemoteConfig();

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

	// Reset draft when dialog opens
	useEffect(() => {
		if (open) {
			setDraft(settings);
			setAppDirty(false);
		}
	}, [open, settings]);

	const updateDraft = useCallback(
		(updater: (d: AppSettings) => AppSettings) => {
			setDraft(updater);
			setAppDirty(true);
		},
		[],
	);

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

	const { isDirty: webhookIsDirty, save: webhookSave } = webhook;
	const { isDirty: remoteIsDirty, save: remoteSave } = remote;

	const handleSave = useCallback(async () => {
		setSaving(true);
		setAppDirty(false);
		try {
			onSave(draft);
			if (webhookIsDirty) {
				await webhookSave();
			}
			if (remoteIsDirty) {
				await remoteSave();
			}
			if (draft.telemetryEnabled) {
				trackEvent("settings_saved");
			}
		} catch {
			// webhook/remote の保存失敗はフック内部でerror stateに反映されUIに表示される
		} finally {
			setSaving(false);
		}
	}, [draft, onSave, webhookIsDirty, webhookSave, remoteIsDirty, remoteSave]);

	const isDirty = appDirty || webhookIsDirty || remoteIsDirty;

	const sectionContent = (() => {
		switch (activeSection) {
			case "appearance":
				return <AppearanceSection draft={draft} updateDraft={updateDraft} />;
			case "editor":
				return <EditorSection draft={draft} updateDraft={updateDraft} />;
			case "agent":
				return (
					<AgentSection
						draft={draft}
						updateDraft={updateDraft}
						hooksConfig={hooksConfig}
						hooksLoading={hooksLoading}
						hooksApplying={hooksApplying}
						hooksStatus={hooksStatus}
						hooksCopied={hooksCopied}
						hooksError={hooksError}
						hooksSuccess={hooksSuccess}
						onApplyHooks={handleApplyHooks}
						onCopyHooks={handleCopyHooks}
					/>
				);
			case "remote":
				return <RemoteSection remote={remote} />;
			case "notifications":
				return <NotificationsSection webhook={webhook} />;
			case "privacy":
				return <PrivacySection draft={draft} updateDraft={updateDraft} />;
		}
	})();

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-3xl h-[70vh] flex flex-col p-0 gap-0">
				<DialogHeader className="px-6 pt-6 pb-0 shrink-0">
					<DialogTitle>Settings</DialogTitle>
					<DialogDescription className="sr-only">
						Application settings
					</DialogDescription>
				</DialogHeader>

				<div className="flex flex-1 min-h-0 border-t border-border mt-4">
					<nav className="w-48 shrink-0 border-r border-border py-2">
						{SETTINGS_SECTIONS.map((section) => {
							const Icon = section.icon;
							return (
								<button
									key={section.id}
									type="button"
									onClick={() => setActiveSection(section.id)}
									className={cn(
										"flex items-center gap-2 w-full px-4 py-1.5 text-sm text-left transition-colors",
										activeSection === section.id
											? "bg-accent text-accent-foreground font-medium"
											: "text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground",
									)}
								>
									<Icon className="size-4" />
									{section.label}
								</button>
							);
						})}
					</nav>

					<ScrollArea className="flex-1 min-h-0">
						<div className="px-6 py-4">{sectionContent}</div>
					</ScrollArea>
				</div>

				<DialogFooter className="px-6 py-4 border-t border-border shrink-0">
					<Button
						type="button"
						size="sm"
						onClick={handleSave}
						disabled={!isDirty || saving}
					>
						{saving ? <Loader2 className="size-3.5 animate-spin" /> : "Save"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
