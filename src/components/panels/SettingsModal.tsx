import { invoke } from "@tauri-apps/api/core";
import {
	Bell,
	Bot,
	Check,
	Code,
	Copy,
	GitBranch,
	Globe,
	Loader2,
	Monitor,
	Palette,
	Shield,
} from "lucide-react";
import { Fragment, useCallback, useEffect, useReducer, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Slider } from "@/components/ui/slider";
import { useBackgroundConfig } from "@/hooks/useAppSettings";
import { useRemoteConfig } from "@/hooks/useRemoteConfig";
import { useWebhookConfig } from "@/hooks/useWebhookConfig";
import { trackEvent } from "@/lib/telemetry";
import { cn } from "@/lib/utils";
import type { BranchInfo } from "@/types/git";
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

interface HooksState {
	config: string;
	loading: boolean;
	applying: boolean;
	status: "active" | "token_mismatch" | "not_configured";
	copied: boolean;
	error: string | null;
	success: boolean;
}

const initialHooksState: HooksState = {
	config: "",
	loading: false,
	applying: false,
	status: "not_configured",
	copied: false,
	error: null,
	success: false,
};

type HooksAction =
	| { type: "LOAD_START" }
	| {
			type: "LOAD_SUCCESS";
			config: string;
			status: HooksState["status"];
	  }
	| { type: "LOAD_ERROR"; error: string }
	| { type: "APPLY_START" }
	| { type: "APPLY_SUCCESS" }
	| { type: "APPLY_ERROR"; error: string }
	| { type: "SET_COPIED"; copied: boolean }
	| { type: "COPY_ERROR"; error: string };

export function hooksReducer(
	state: HooksState,
	action: HooksAction,
): HooksState {
	switch (action.type) {
		case "LOAD_START":
			return { ...state, loading: true, error: null, success: false };
		case "LOAD_SUCCESS":
			return {
				...state,
				loading: false,
				error: null,
				config: action.config,
				status: action.status,
			};
		case "LOAD_ERROR":
			return { ...state, loading: false, error: action.error };
		case "APPLY_START":
			return { ...state, applying: true, error: null, success: false };
		case "APPLY_SUCCESS":
			return {
				...state,
				applying: false,
				status: "active",
				success: true,
				error: null,
			};
		case "APPLY_ERROR":
			return { ...state, applying: false, error: action.error };
		case "SET_COPIED":
			return { ...state, copied: action.copied };
		case "COPY_ERROR":
			return { ...state, error: action.error };
	}
}

type SettingsSection =
	| "appearance"
	| "editor"
	| "repositories"
	| "agent"
	| "remote"
	| "background"
	| "notifications"
	| "privacy";

const SETTINGS_SECTIONS: {
	id: SettingsSection;
	label: string;
	icon: React.ComponentType<{ className?: string }>;
}[] = [
	{ id: "appearance", label: "Appearance", icon: Palette },
	{ id: "editor", label: "Editor", icon: Code },
	{ id: "repositories", label: "Repositories", icon: GitBranch },
	{ id: "agent", label: "Agent", icon: Bot },
	{ id: "remote", label: "Remote", icon: Globe },
	{ id: "background", label: "Background", icon: Monitor },
	{ id: "notifications", label: "Notifications", icon: Bell },
	{ id: "privacy", label: "Privacy & Updates", icon: Shield },
];

const labelClass = "text-xs font-medium text-muted-foreground";

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
				<Select
					value={draft.theme}
					onValueChange={(value) =>
						updateDraft((d) => ({ ...d, theme: value as Theme }))
					}
				>
					<SelectTrigger id="theme-select">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="dark">Dark</SelectItem>
						<SelectItem value="light">Light</SelectItem>
					</SelectContent>
				</Select>
			</div>

			<div className="flex flex-col gap-1.5">
				<label htmlFor="font-size-slider" className={labelClass}>
					Font Size: {draft.fontSize}px
				</label>
				<Slider
					id="font-size-slider"
					min={12}
					max={24}
					step={1}
					value={[draft.fontSize]}
					onValueChange={([v]) => updateDraft((d) => ({ ...d, fontSize: v }))}
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
				<Select
					value={draft.defaultDiffBase}
					onValueChange={(value) =>
						updateDraft((d) => ({
							...d,
							defaultDiffBase: value as DiffBase,
						}))
					}
				>
					<SelectTrigger id="diff-base-select">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="staged">Staged</SelectItem>
						<SelectItem value="HEAD">HEAD</SelectItem>
					</SelectContent>
				</Select>
			</div>

			<div className="flex flex-col gap-1.5">
				<label htmlFor="diff-mode-select" className={labelClass}>
					Default View
				</label>
				<Select
					value={draft.defaultDiffMode}
					onValueChange={(value) =>
						updateDraft((d) => ({
							...d,
							defaultDiffMode: value as DiffMode,
						}))
					}
				>
					<SelectTrigger id="diff-mode-select">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="gutter">Gutter</SelectItem>
						<SelectItem value="inline">Inline</SelectItem>
						<SelectItem value="split">Split</SelectItem>
					</SelectContent>
				</Select>
			</div>
		</div>
	);
}

function RepoBaseBranchItem({ repoPath }: { repoPath: string }) {
	const [branches, setBranches] = useState<BranchInfo[]>([]);
	const [selectedBase, setSelectedBase] = useState("");
	const [initialBase, setInitialBase] = useState("");
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [success, setSuccess] = useState(false);

	useEffect(() => {
		setLoading(true);
		setError(null);
		setSuccess(false);

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
			})
			.finally(() => {
				setLoading(false);
			});
	}, [repoPath]);

	const handleApply = useCallback(async () => {
		setSaving(true);
		setError(null);
		setSuccess(false);
		try {
			await invoke("set_releash_base", {
				repoPath,
				base: selectedBase || null,
			});
			setInitialBase(selectedBase);
			setSuccess(true);
		} catch (e) {
			setError(String(e));
		} finally {
			setSaving(false);
		}
	}, [repoPath, selectedBase]);

	const isDirty = selectedBase !== initialBase;
	const name = repoPath.split(/[\\/]/).pop() ?? repoPath;
	const selectId = `base-branch-${name}`;

	return (
		<div>
			<div className="flex items-center gap-2 px-3 py-2 text-sm font-medium">
				<GitBranch className="size-3.5 shrink-0 text-muted-foreground" />
				<span className="font-mono truncate">{name}</span>
			</div>
			<div className="ml-5 pl-3">
				{loading ? (
					<div className="flex items-center justify-center py-3">
						<Loader2 className="size-4 animate-spin text-muted-foreground" />
					</div>
				) : (
					<div className="flex flex-col gap-1.5">
						<label htmlFor={selectId} className={labelClass}>
							Base branch
						</label>
						<Select
							value={selectedBase || "__auto__"}
							onValueChange={(v) => setSelectedBase(v === "__auto__" ? "" : v)}
						>
							<SelectTrigger id={selectId} className="w-full font-mono">
								<SelectValue placeholder="Auto (main/master)" />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="__auto__">Auto (main/master)</SelectItem>
								{branches.map((b) => (
									<SelectItem key={b.name} value={b.name}>
										{b.name}
									</SelectItem>
								))}
							</SelectContent>
						</Select>

						{error && <p className="text-xs text-destructive">{error}</p>}
						{success && (
							<p className="text-xs text-success">Base branch saved.</p>
						)}

						<div className="flex justify-end mt-1">
							<Button
								size="sm"
								onClick={handleApply}
								disabled={!isDirty || saving}
							>
								{saving ? (
									<Loader2 className="size-3.5 animate-spin" />
								) : (
									"Apply"
								)}
							</Button>
						</div>
					</div>
				)}
			</div>
		</div>
	);
}

function RepositoriesSection({ repoPaths }: { repoPaths: string[] }) {
	if (repoPaths.length === 0) {
		return (
			<p className="text-xs text-muted-foreground">
				No repositories registered.
			</p>
		);
	}

	return (
		<div className="flex flex-col">
			{repoPaths.map((repoPath, i) => (
				<Fragment key={repoPath}>
					{i > 0 && <Separator className="my-3" />}
					<RepoBaseBranchItem repoPath={repoPath} />
				</Fragment>
			))}
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
				<Select
					value={draft.agent}
					onValueChange={(value) =>
						updateDraft((d) => ({
							...d,
							agent: value as AgentType,
						}))
					}
				>
					<SelectTrigger id="agent-select">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{AGENT_TYPE_KEYS.map((key) => (
							<SelectItem key={key} value={key}>
								{AGENT_CONFIGS[key].label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>

			{showAutoApprove && (
				<div className="flex items-center gap-2">
					<Checkbox
						id="agent-auto-approve"
						checked={draft.agentAutoApprove}
						onCheckedChange={(checked) =>
							updateDraft((d) => ({
								...d,
								agentAutoApprove: checked === true,
							}))
						}
					/>
					<label
						htmlFor="agent-auto-approve"
						className={`${labelClass} cursor-pointer`}
					>
						Auto-approve
					</label>
				</div>
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
											Token mismatch — Reconfiguration required
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
									Settings applied. Restart Claude Code to take effect.
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
									{hooksStatus === "active" ? "Reconfigure" : "Apply Settings"}
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
					<div className="flex items-center gap-2">
						<Checkbox
							id="remote-auto-start"
							checked={remote.draft.auto_start}
							onCheckedChange={(checked) =>
								remote.setDraft((d) => ({
									...d,
									auto_start: checked === true,
									auto_start_on_lan:
										checked === true ? d.auto_start_on_lan : false,
								}))
							}
						/>
						<label
							htmlFor="remote-auto-start"
							className={`${labelClass} cursor-pointer`}
						>
							Auto-start remote server
						</label>
					</div>

					<div
						className={`flex items-center gap-2 ml-4 ${remote.draft.auto_start ? "" : "cursor-not-allowed opacity-50"}`}
					>
						<Checkbox
							id="remote-auto-start-lan"
							checked={remote.draft.auto_start_on_lan}
							disabled={!remote.draft.auto_start}
							onCheckedChange={(checked) =>
								remote.setDraft((d) => ({
									...d,
									auto_start_on_lan: checked === true,
								}))
							}
						/>
						<label
							htmlFor="remote-auto-start-lan"
							className={`${labelClass} ${remote.draft.auto_start ? "cursor-pointer" : ""}`}
						>
							Allow auto-start on LAN
						</label>
					</div>

					<p className="text-[10px] text-muted-foreground">
						Auto-starts on VPN connection. Auto-start on LAN can be controlled
						above.
					</p>

					{remote.error && (
						<p className="text-xs text-destructive">{remote.error}</p>
					)}
				</>
			)}
		</div>
	);
}

function BackgroundSection({
	background,
}: {
	background: ReturnType<typeof useBackgroundConfig>;
}) {
	return (
		<div className="flex flex-col gap-2">
			{background.loading ? (
				<div className="flex items-center justify-center py-4">
					<Loader2 className="size-4 animate-spin text-muted-foreground" />
				</div>
			) : (
				<>
					<div className="flex items-center gap-2">
						<Checkbox
							id="close-to-tray"
							checked={background.draft.close_to_tray}
							onCheckedChange={(checked) =>
								background.setDraft((d) => ({
									...d,
									close_to_tray: checked === true,
								}))
							}
						/>
						<label
							htmlFor="close-to-tray"
							className={`${labelClass} cursor-pointer`}
						>
							Minimize to tray on close
						</label>
					</div>
					<p className="text-[10px] text-muted-foreground ml-6 -mt-1">
						{background.draft.close_to_tray
							? "Window hides to tray; restore from tray icon."
							: "Window minimizes to Dock/taskbar."}
					</p>

					<div className="flex items-center gap-2">
						<Checkbox
							id="auto-launch"
							checked={background.draft.auto_launch}
							onCheckedChange={(checked) =>
								background.setDraft((d) => ({
									...d,
									auto_launch: checked === true,
								}))
							}
						/>
						<label
							htmlFor="auto-launch"
							className={`${labelClass} cursor-pointer`}
						>
							Launch at login
						</label>
					</div>

					<div
						className={`flex items-center gap-2 ml-4 ${background.draft.auto_launch ? "" : "cursor-not-allowed opacity-50"}`}
					>
						<Checkbox
							id="start-minimized"
							checked={background.draft.start_minimized}
							disabled={!background.draft.auto_launch}
							onCheckedChange={(checked) =>
								background.setDraft((d) => ({
									...d,
									start_minimized: checked === true,
								}))
							}
						/>
						<label
							htmlFor="start-minimized"
							className={`${labelClass} ${background.draft.auto_launch ? "cursor-pointer" : ""}`}
						>
							Start minimized
						</label>
					</div>

					{background.error && (
						<p className="text-xs text-destructive">{background.error}</p>
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
								<div key={key} className="flex items-center gap-1">
									<Checkbox
										id={`notify-${key}`}
										checked={webhook.draft[key]}
										onCheckedChange={(checked) =>
											webhook.setDraft((d) => ({
												...d,
												[key]: checked === true,
											}))
										}
									/>
									<label
										htmlFor={`notify-${key}`}
										className="text-xs cursor-pointer"
									>
										{label}
									</label>
								</div>
							))}
						</div>
					</div>

					<div className="flex flex-col gap-1.5">
						<span className={labelClass}>Send notifications</span>
						<RadioGroup
							value={webhook.draft.desktop_mode}
							onValueChange={(value) =>
								webhook.setDraft((d) => ({
									...d,
									desktop_mode: value as DesktopNotifyMode,
								}))
							}
						>
							<div className="flex items-center gap-2">
								<RadioGroupItem value="always" id="desktop-always" />
								<label
									htmlFor="desktop-always"
									className="text-xs cursor-pointer"
								>
									Always
								</label>
							</div>
							<div className="flex items-center gap-2">
								<RadioGroupItem
									value="when_inactive"
									id="desktop-when-inactive"
								/>
								<label
									htmlFor="desktop-when-inactive"
									className="text-xs cursor-pointer"
								>
									When inactive for
								</label>
								{webhook.draft.desktop_mode === "when_inactive" && (
									<Select
										value={String(webhook.draft.inactive_timeout_minutes)}
										onValueChange={(value) =>
											webhook.setDraft((d) => ({
												...d,
												inactive_timeout_minutes: Number(value),
											}))
										}
									>
										<SelectTrigger className="w-auto min-w-[80px]">
											<SelectValue />
										</SelectTrigger>
										<SelectContent>
											{INACTIVE_TIMEOUT_OPTIONS.map((opt) => (
												<SelectItem key={opt.value} value={String(opt.value)}>
													{opt.label}
												</SelectItem>
											))}
										</SelectContent>
									</Select>
								)}
							</div>
						</RadioGroup>
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
			<div className="flex items-center gap-2">
				<Checkbox
					id="auto-update"
					checked={draft.autoUpdate}
					onCheckedChange={(checked) =>
						updateDraft((d) => ({
							...d,
							autoUpdate: checked === true,
						}))
					}
				/>
				<label htmlFor="auto-update" className={`${labelClass} cursor-pointer`}>
					Auto-update
				</label>
			</div>

			<div className="flex items-center gap-2">
				<Checkbox
					id="telemetry-enabled"
					checked={draft.telemetryEnabled}
					onCheckedChange={(checked) =>
						updateDraft((d) => ({
							...d,
							telemetryEnabled: checked === true,
						}))
					}
				/>
				<label
					htmlFor="telemetry-enabled"
					className={`${labelClass} cursor-pointer`}
				>
					Send anonymous usage data
				</label>
			</div>

			<div className="flex items-center gap-2">
				<Checkbox
					id="crash-reporting"
					checked={draft.enableCrashReporting}
					onCheckedChange={(checked) =>
						updateDraft((d) => ({
							...d,
							enableCrashReporting: checked === true,
						}))
					}
				/>
				<label
					htmlFor="crash-reporting"
					className={`${labelClass} cursor-pointer`}
				>
					Send crash reports
				</label>
			</div>
			<p className="text-[10px] text-muted-foreground -mt-2">
				Help improve Releash by sending anonymous crash reports.
			</p>
		</div>
	);
}

interface SettingsState {
	activeSection: SettingsSection;
	draft: AppSettings;
	appDirty: boolean;
	saving: boolean;
	prevOpen: boolean;
}

type SettingsAction =
	| { type: "SET_SECTION"; section: SettingsSection }
	| { type: "UPDATE_DRAFT"; updater: (d: AppSettings) => AppSettings }
	| { type: "SYNC_OPEN"; open: boolean; settings: AppSettings }
	| { type: "SAVE_START" }
	| { type: "SAVE_END" }
	| { type: "SAVE_ERROR" };

export function settingsReducer(
	state: SettingsState,
	action: SettingsAction,
): SettingsState {
	switch (action.type) {
		case "SET_SECTION":
			return { ...state, activeSection: action.section };
		case "UPDATE_DRAFT":
			return { ...state, draft: action.updater(state.draft), appDirty: true };
		case "SYNC_OPEN":
			if (action.open && !state.prevOpen) {
				return {
					...state,
					prevOpen: action.open,
					draft: action.settings,
					appDirty: false,
				};
			}
			return { ...state, prevOpen: action.open };
		case "SAVE_START":
			return { ...state, saving: true, appDirty: false };
		case "SAVE_END":
			return { ...state, saving: false };
		case "SAVE_ERROR":
			return { ...state, saving: false, appDirty: true };
	}
}

export interface SettingsModalProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	settings: AppSettings;
	onSave: (settings: AppSettings) => void;
	repoPaths?: string[];
}

export function SettingsModal({
	open,
	onOpenChange,
	settings,
	onSave,
	repoPaths = [],
}: SettingsModalProps) {
	const [state, dispatchSettings] = useReducer(settingsReducer, {
		activeSection: "appearance" as SettingsSection,
		draft: settings,
		appDirty: false,
		saving: false,
		prevOpen: open,
	});
	const { activeSection, draft, appDirty, saving } = state;
	const webhook = useWebhookConfig();
	const remote = useRemoteConfig();
	const background = useBackgroundConfig();

	// Hooks state
	const [hooks, dispatchHooks] = useReducer(hooksReducer, initialHooksState);

	// Reset draft when dialog opens
	if (open !== state.prevOpen) {
		dispatchSettings({ type: "SYNC_OPEN", open, settings });
		if (open && settings.agent === "claude") {
			dispatchHooks({ type: "LOAD_START" });
		}
	}

	const updateDraft = useCallback(
		(updater: (d: AppSettings) => AppSettings) => {
			dispatchSettings({ type: "UPDATE_DRAFT", updater });
		},
		[],
	);

	// Load hooks config when agent is claude
	useEffect(() => {
		if (draft.agent !== "claude") return;
		dispatchHooks({ type: "LOAD_START" });

		Promise.all([
			invoke<string>("generate_hooks_config"),
			invoke<string>("get_hooks_status"),
		])
			.then(([json, status]) => {
				dispatchHooks({
					type: "LOAD_SUCCESS",
					config: json,
					status: status as HooksState["status"],
				});
			})
			.catch((e) => {
				dispatchHooks({ type: "LOAD_ERROR", error: String(e) });
			});
	}, [draft.agent]);

	const handleApplyHooks = useCallback(async () => {
		dispatchHooks({ type: "APPLY_START" });
		try {
			await invoke("apply_hooks_config", { configJson: hooks.config });
			dispatchHooks({ type: "APPLY_SUCCESS" });
		} catch (e) {
			dispatchHooks({ type: "APPLY_ERROR", error: String(e) });
		}
	}, [hooks.config]);

	const handleCopyHooks = useCallback(async () => {
		try {
			await navigator.clipboard.writeText(hooks.config);
			dispatchHooks({ type: "SET_COPIED", copied: true });
			setTimeout(
				() => dispatchHooks({ type: "SET_COPIED", copied: false }),
				2000,
			);
		} catch (e) {
			dispatchHooks({ type: "COPY_ERROR", error: `Copy failed: ${String(e)}` });
		}
	}, [hooks.config]);

	const { isDirty: webhookIsDirty, save: webhookSave } = webhook;
	const { isDirty: remoteIsDirty, save: remoteSave } = remote;
	const { isDirty: backgroundIsDirty, save: backgroundSave } = background;

	const handleSave = useCallback(async () => {
		dispatchSettings({ type: "SAVE_START" });
		try {
			onSave(draft);
			if (webhookIsDirty) {
				await webhookSave();
			}
			if (remoteIsDirty) {
				await remoteSave();
			}
			if (backgroundIsDirty) {
				await backgroundSave();
			}
			if (draft.telemetryEnabled) {
				trackEvent("settings_saved");
			}
		} catch {
			// webhook/remote の保存失敗はフック内部でerror stateに反映されUIに表示される
			dispatchSettings({ type: "SAVE_ERROR" });
		} finally {
			dispatchSettings({ type: "SAVE_END" });
		}
	}, [
		draft,
		onSave,
		webhookIsDirty,
		webhookSave,
		remoteIsDirty,
		remoteSave,
		backgroundIsDirty,
		backgroundSave,
	]);

	const isDirty =
		appDirty || webhookIsDirty || remoteIsDirty || backgroundIsDirty;

	const sectionContent = (() => {
		switch (activeSection) {
			case "appearance":
				return <AppearanceSection draft={draft} updateDraft={updateDraft} />;
			case "editor":
				return <EditorSection draft={draft} updateDraft={updateDraft} />;
			case "repositories":
				return <RepositoriesSection repoPaths={repoPaths} />;
			case "agent":
				return (
					<AgentSection
						draft={draft}
						updateDraft={updateDraft}
						hooksConfig={hooks.config}
						hooksLoading={hooks.loading}
						hooksApplying={hooks.applying}
						hooksStatus={hooks.status}
						hooksCopied={hooks.copied}
						hooksError={hooks.error}
						hooksSuccess={hooks.success}
						onApplyHooks={handleApplyHooks}
						onCopyHooks={handleCopyHooks}
					/>
				);
			case "remote":
				return <RemoteSection remote={remote} />;
			case "background":
				return <BackgroundSection background={background} />;
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
									onClick={() =>
										dispatchSettings({
											type: "SET_SECTION",
											section: section.id,
										})
									}
									className={cn(
										"flex items-center gap-2 w-full px-4 py-1.5 text-sm text-left transition-colors",
										activeSection === section.id
											? "bg-muted text-foreground font-medium"
											: "text-muted-foreground hover:bg-secondary hover:text-foreground",
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
