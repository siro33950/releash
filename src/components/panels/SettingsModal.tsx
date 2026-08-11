import { invoke } from "@tauri-apps/api/core";
import {
	Bell,
	BookOpen,
	Bot,
	Code,
	GitBranch,
	Loader2,
	Monitor,
	Palette,
	Shield,
	Trash2,
	Workflow,
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
import { useAutomation } from "@/hooks/useAutomation";
import { useNotionSettings } from "@/hooks/useNotionSettings";
import { useProviderAvailabilitySettings } from "@/hooks/useProviderAvailabilitySettings";
import { useWebhookConfig } from "@/hooks/useWebhookConfig";
import { setPerformanceTelemetryEnabled, trackEvent } from "@/lib/telemetry";
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
import { AutomationSection } from "./AutomationSection";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import { NotionSettingsSection } from "./NotionSettingsSection";
import { ProviderAvailabilitySettings } from "./ProviderAvailabilitySettings";

const AGENT_TYPE_KEYS = Object.keys(AGENT_CONFIGS) as AgentType[];

interface WorkflowConfig {
	approval_auto_approve: boolean;
}

const DEFAULT_WORKFLOW_CONFIG: WorkflowConfig = {
	approval_auto_approve: false,
};

function useWorkflowSettings(open: boolean) {
	const [config, setConfig] = useState<WorkflowConfig>(DEFAULT_WORKFLOW_CONFIG);
	const [draft, setDraft] = useState<WorkflowConfig>(DEFAULT_WORKFLOW_CONFIG);
	const [loading, setLoading] = useState(false);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (!open) return;
		let cancelled = false;
		setLoading(true);
		setError(null);
		invoke<WorkflowConfig>("get_workflow_config")
			.then((loaded) => {
				if (cancelled) return;
				const normalized = loaded ?? DEFAULT_WORKFLOW_CONFIG;
				setConfig(normalized);
				setDraft(normalized);
			})
			.catch((e) => {
				if (!cancelled) setError(String(e));
			})
			.finally(() => {
				if (!cancelled) setLoading(false);
			});
		return () => {
			cancelled = true;
		};
	}, [open]);

	const isDirty = JSON.stringify(draft) !== JSON.stringify(config);

	const save = useCallback(async () => {
		setSaving(true);
		setError(null);
		try {
			await invoke("update_workflow_config", { workflow: draft });
			setConfig({ ...draft });
		} catch (e) {
			setError(String(e));
			throw e;
		} finally {
			setSaving(false);
		}
	}, [draft]);

	return { draft, setDraft, isDirty, loading, saving, error, save };
}

type SettingsSection =
	| "appearance"
	| "editor"
	| "repositories"
	| "notion"
	| "agent"
	| "background"
	| "notifications"
	| "automation"
	| "privacy";

const SETTINGS_SECTIONS: {
	id: SettingsSection;
	label: string;
	icon: React.ComponentType<{ className?: string }>;
}[] = [
	{ id: "appearance", label: "Appearance", icon: Palette },
	{ id: "editor", label: "Editor", icon: Code },
	{ id: "repositories", label: "Repositories", icon: GitBranch },
	{ id: "notion", label: "Notion", icon: BookOpen },
	{ id: "agent", label: "Agent", icon: Bot },
	{ id: "background", label: "Background", icon: Monitor },
	{ id: "notifications", label: "Notifications", icon: Bell },
	{ id: "automation", label: "Automation", icon: Workflow },
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

interface EditorInfo {
	name: string;
	path: string;
}

function useExternalEditorConfig(open: boolean) {
	const [editor, setEditor] = useState("");
	const [initialEditor, setInitialEditor] = useState("");
	const [editors, setEditors] = useState<EditorInfo[]>([]);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (!open) return;
		let cancelled = false;
		setLoading(true);
		setError(null);
		Promise.all([
			invoke<string>("get_external_editor"),
			invoke<EditorInfo[]>("detect_editors"),
		])
			.then(([current, detected]) => {
				if (cancelled) return;
				setEditor(current);
				setInitialEditor(current);
				setEditors(detected);
			})
			.catch((e) => {
				if (!cancelled) setError(String(e));
			})
			.finally(() => {
				if (!cancelled) setLoading(false);
			});
		return () => {
			cancelled = true;
		};
	}, [open]);

	const isDirty = editor !== initialEditor;

	const save = useCallback(async () => {
		setError(null);
		try {
			await invoke("update_external_editor", { editor });
			setInitialEditor(editor);
		} catch (e) {
			setError(String(e));
			throw e;
		}
	}, [editor]);

	return { editor, setEditor, editors, isDirty, loading, error, save };
}

function EditorSection({
	draft,
	updateDraft,
	externalEditor,
}: {
	draft: AppSettings;
	updateDraft: (updater: (d: AppSettings) => AppSettings) => void;
	externalEditor: ReturnType<typeof useExternalEditorConfig>;
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
						<SelectItem value="head">HEAD</SelectItem>
						<SelectItem value="branch-base">Branch Base</SelectItem>
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

			<div className="flex items-center gap-2">
				<Checkbox
					id="diff-only-mode"
					checked={draft.defaultDiffOnlyMode}
					onCheckedChange={(checked) =>
						updateDraft((d) => ({
							...d,
							defaultDiffOnlyMode: checked === true,
						}))
					}
				/>
				<label htmlFor="diff-only-mode" className={labelClass}>
					Show diff only by default
				</label>
			</div>

			<div className="flex flex-col gap-1.5">
				<label htmlFor="external-editor-select" className={labelClass}>
					External Editor
				</label>
				{externalEditor.loading ? (
					<div className="flex items-center justify-center py-3">
						<Loader2 className="size-4 animate-spin text-muted-foreground" />
					</div>
				) : (
					<>
						<Select
							value={externalEditor.editor || "__default__"}
							onValueChange={(value) =>
								externalEditor.setEditor(value === "__default__" ? "" : value)
							}
						>
							<SelectTrigger id="external-editor-select">
								<SelectValue placeholder="System Default" />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="__default__">System Default</SelectItem>
								{externalEditor.editors.map((e) => (
									<SelectItem key={e.path} value={e.path}>
										{e.name}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						<p className="text-[10px] text-muted-foreground">
							Application used when opening files from the diff view.
						</p>
						{externalEditor.error && (
							<p className="text-xs text-destructive">{externalEditor.error}</p>
						)}
					</>
				)}
			</div>
		</div>
	);
}

function RepoBaseBranchItem({
	repoPath,
	onDirtyChange,
}: {
	repoPath: string;
	onDirtyChange: (
		repoPath: string,
		isDirty: boolean,
		selectedBase: string,
	) => void;
}) {
	const [branches, setBranches] = useState<BranchInfo[]>([]);
	const [selectedBase, setSelectedBase] = useState("");
	const [initialBase, setInitialBase] = useState("");
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		setLoading(true);
		setError(null);

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

	const handleChange = useCallback(
		(v: string) => {
			const value = v === "__auto__" ? "" : v;
			setSelectedBase(value);
			onDirtyChange(repoPath, value !== initialBase, value);
		},
		[repoPath, initialBase, onDirtyChange],
	);

	const name = repoPath.split(/[\\/]/).pop() ?? repoPath;
	const selectId = `base-branch-${repoPath.replace(/\//g, "_")}`;

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
							onValueChange={handleChange}
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
					</div>
				)}
			</div>
		</div>
	);
}

interface RepoChanges {
	pendingBases: Map<string, string>;
	isDirty: boolean;
	error: string | null;
	revision: number;
}

function useRepoChanges() {
	const [state, setState] = useState<RepoChanges>({
		pendingBases: new Map(),
		isDirty: false,
		error: null,
		revision: 0,
	});

	const handleDirtyChange = useCallback(
		(repoPath: string, isDirty: boolean, selectedBase: string) => {
			setState((prev) => {
				const next = new Map(prev.pendingBases);
				if (isDirty) {
					next.set(repoPath, selectedBase);
				} else {
					next.delete(repoPath);
				}
				return { ...prev, pendingBases: next, isDirty: next.size > 0 };
			});
		},
		[],
	);

	const save = useCallback(async () => {
		const entries = Array.from(state.pendingBases.entries());
		setState((prev) => ({ ...prev, error: null }));
		try {
			await Promise.all(
				entries.map(([repoPath, base]) =>
					invoke("set_releash_base", { repoPath, base: base || null }),
				),
			);
			setState((prev) => ({
				pendingBases: new Map(),
				isDirty: false,
				error: null,
				revision: prev.revision + 1,
			}));
		} catch (e) {
			setState((prev) => ({ ...prev, error: String(e) }));
			throw e;
		}
	}, [state.pendingBases]);

	const reset = useCallback(() => {
		setState((prev) => ({
			pendingBases: new Map(),
			isDirty: false,
			error: null,
			revision: prev.revision + 1,
		}));
	}, []);

	return { ...state, handleDirtyChange, save, reset };
}

function RepositoriesSection({
	repoPaths,
	onDirtyChange,
	error,
	revision,
	onRemoveRepo,
}: {
	repoPaths: string[];
	onDirtyChange: (
		repoPath: string,
		isDirty: boolean,
		selectedBase: string,
	) => void;
	error: string | null;
	revision: number;
	onRemoveRepo?: (path: string) => void;
}) {
	const [removeTarget, setRemoveTarget] = useState<string | null>(null);

	if (repoPaths.length === 0) {
		return (
			<p className="text-xs text-muted-foreground">
				No repositories registered.
			</p>
		);
	}

	return (
		<div className="flex flex-col">
			{error && <p className="text-xs text-destructive">{error}</p>}
			{repoPaths.map((repoPath, i) => (
				<Fragment key={`${repoPath}-${revision}`}>
					{i > 0 && <Separator className="my-3" />}
					<div className="flex items-start gap-1">
						<div className="flex-1 min-w-0">
							<RepoBaseBranchItem
								repoPath={repoPath}
								onDirtyChange={onDirtyChange}
							/>
						</div>
						{onRemoveRepo && (
							<Button
								variant="ghost"
								size="icon"
								className="size-7 mt-1.5 shrink-0 text-destructive hover:text-destructive"
								onClick={() => setRemoveTarget(repoPath)}
								aria-label={`Remove repository ${repoPath.split(/[\\/]/).pop() ?? repoPath}`}
								title="Remove repository"
							>
								<Trash2 className="size-3.5" />
							</Button>
						)}
					</div>
				</Fragment>
			))}
			{onRemoveRepo && removeTarget && (
				<DeleteConfirmDialog
					open={true}
					itemName={removeTarget.split(/[\\/]/).pop() ?? removeTarget}
					description="Remove from list? The repository will not be deleted from disk."
					onConfirm={() => {
						onRemoveRepo(removeTarget);
						setRemoveTarget(null);
					}}
					onCancel={() => setRemoveTarget(null)}
				/>
			)}
		</div>
	);
}

function AgentSection({
	draft,
	updateDraft,
	workflow,
	providerAvailability,
}: {
	draft: AppSettings;
	updateDraft: (updater: (d: AppSettings) => AppSettings) => void;
	workflow: ReturnType<typeof useWorkflowSettings>;
	providerAvailability: ReturnType<typeof useProviderAvailabilitySettings>;
}) {
	const showAutoApprove =
		draft.agent !== "none" &&
		draft.agent !== "cursor" &&
		draft.agent !== "custom";

	return (
		<div className="flex flex-col gap-4">
			<ProviderAvailabilitySettings settings={providerAvailability} />

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

			<div className="flex flex-col gap-1.5 rounded border p-3">
				<div className="flex items-center gap-2">
					<Checkbox
						id="workflow-approval-auto-approve"
						checked={workflow.draft.approval_auto_approve}
						disabled={workflow.loading || workflow.saving}
						onCheckedChange={(checked) =>
							workflow.setDraft((d) => ({
								...d,
								approval_auto_approve: checked === true,
							}))
						}
					/>
					<label
						htmlFor="workflow-approval-auto-approve"
						className={`${labelClass} cursor-pointer`}
					>
						Approval gate auto-approve
					</label>
				</div>
				<p className="text-[10px] text-muted-foreground">
					Automatically approves completed sessions with gate: approval. This is
					independent from agent auto-approve.
				</p>
				{workflow.error && (
					<p className="text-[10px] text-destructive">{workflow.error}</p>
				)}
			</div>

			{draft.agent !== "none" && (
				<div className="flex flex-col gap-1.5">
					<label htmlFor="agent-max-concurrent" className={labelClass}>
						Max concurrent agent PTYs
					</label>
					<input
						id="agent-max-concurrent"
						type="number"
						min={0}
						value={draft.agentMaxConcurrent}
						onChange={(e) =>
							updateDraft((d) => ({
								...d,
								agentMaxConcurrent: Math.max(0, Number(e.target.value) || 0),
							}))
						}
						className="w-24 bg-muted border border-border rounded px-2 py-1.5 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
					/>
					<p className="text-[10px] text-muted-foreground">
						Limits how many agent sessions are pre-spawned at startup. 0 =
						unlimited.
					</p>
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

// URL の hostname を厳密一致で判定する（substring 判定は
// `https://evil.com/discord.com/api/webhooks/` 等で回避可能なため使わない）。
function detectWebhookType(
	raw: string,
): "Discord" | "Slack" | "Generic (Slack format)" | null {
	if (!raw) return null;
	let host: string;
	let pathname: string;
	try {
		const url = new URL(raw);
		host = url.hostname.toLowerCase();
		pathname = url.pathname;
	} catch {
		return "Generic (Slack format)";
	}
	if (
		(host === "discord.com" || host === "discordapp.com") &&
		pathname.startsWith("/api/webhooks/")
	) {
		return "Discord";
	}
	if (host === "hooks.slack.com") {
		return "Slack";
	}
	return "Generic (Slack format)";
}

function NotificationsSection({
	webhook,
}: {
	webhook: ReturnType<typeof useWebhookConfig>;
}) {
	const webhookUrlValue = webhook.draft.webhook_url;
	const detectedWebhookType = detectWebhookType(webhookUrlValue);

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
					id="performance-telemetry"
					checked={draft.performanceTelemetry}
					onCheckedChange={(checked) =>
						updateDraft((d) => ({
							...d,
							performanceTelemetry: checked === true,
						}))
					}
				/>
				<label
					htmlFor="performance-telemetry"
					className={`${labelClass} cursor-pointer`}
				>
					Send anonymous performance metrics
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

function settingsReducer(
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
	onRemoveRepo?: (path: string) => void;
}

export function SettingsModal({
	open,
	onOpenChange,
	settings,
	onSave,
	repoPaths = [],
	onRemoveRepo,
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
	const background = useBackgroundConfig();
	const repos = useRepoChanges();
	const notion = useNotionSettings(repoPaths);
	const externalEditor = useExternalEditorConfig(open);
	const automation = useAutomation(open);
	const workflow = useWorkflowSettings(open);
	const providerAvailability = useProviderAvailabilitySettings(open);

	// Reset draft when dialog opens
	if (open !== state.prevOpen) {
		dispatchSettings({ type: "SYNC_OPEN", open, settings });
		if (open) {
			repos.reset();
			notion.reset();
		}
	}

	const updateDraft = useCallback(
		(updater: (d: AppSettings) => AppSettings) => {
			dispatchSettings({ type: "UPDATE_DRAFT", updater });
		},
		[],
	);

	const { isDirty: webhookIsDirty, save: webhookSave } = webhook;
	const { isDirty: backgroundIsDirty, save: backgroundSave } = background;
	const { isDirty: reposIsDirty, save: reposSave } = repos;
	const { isDirty: notionIsDirty, save: notionSave } = notion;
	const { isDirty: editorIsDirty, save: editorSave } = externalEditor;
	const { isDirty: workflowIsDirty, save: workflowSave } = workflow;
	const {
		isDirty: providerAvailabilityIsDirty,
		save: providerAvailabilitySave,
	} = providerAvailability;

	const handleSave = useCallback(async () => {
		dispatchSettings({ type: "SAVE_START" });
		try {
			const performanceTelemetryChanged =
				draft.performanceTelemetry !== settings.performanceTelemetry;
			onSave(draft);
			if (webhookIsDirty) {
				await webhookSave();
			}
			if (backgroundIsDirty) {
				await backgroundSave();
			}
			if (reposIsDirty) {
				await reposSave();
			}
			if (notionIsDirty) {
				await notionSave();
			}
			if (editorIsDirty) {
				await editorSave();
			}
			if (workflowIsDirty) {
				await workflowSave();
			}
			if (providerAvailabilityIsDirty) {
				await providerAvailabilitySave();
			}
			if (performanceTelemetryChanged) {
				await setPerformanceTelemetryEnabled(draft.performanceTelemetry);
			}
			trackEvent("settings_saved");
		} catch {
			// webhook などの保存失敗はフック内部でerror stateに反映されUIに表示される
			dispatchSettings({ type: "SAVE_ERROR" });
		} finally {
			dispatchSettings({ type: "SAVE_END" });
		}
	}, [
		draft,
		onSave,
		settings.performanceTelemetry,
		webhookIsDirty,
		webhookSave,
		backgroundIsDirty,
		backgroundSave,
		reposIsDirty,
		reposSave,
		notionIsDirty,
		notionSave,
		editorIsDirty,
		editorSave,
		workflowIsDirty,
		workflowSave,
		providerAvailabilityIsDirty,
		providerAvailabilitySave,
	]);

	const isDirty =
		appDirty ||
		webhookIsDirty ||
		backgroundIsDirty ||
		reposIsDirty ||
		notionIsDirty ||
		editorIsDirty ||
		workflowIsDirty ||
		providerAvailabilityIsDirty;

	const sectionContent = (() => {
		switch (activeSection) {
			case "appearance":
				return <AppearanceSection draft={draft} updateDraft={updateDraft} />;
			case "editor":
				return (
					<EditorSection
						draft={draft}
						updateDraft={updateDraft}
						externalEditor={externalEditor}
					/>
				);
			case "repositories":
				return (
					<RepositoriesSection
						repoPaths={repoPaths}
						onDirtyChange={repos.handleDirtyChange}
						error={repos.error}
						revision={repos.revision}
						onRemoveRepo={onRemoveRepo}
					/>
				);
			case "notion":
				return (
					<NotionSettingsSection
						repoPaths={repoPaths}
						drafts={notion.drafts}
						updateDraft={notion.updateDraft}
						validate={notion.validate}
						markForDelete={notion.markForDelete}
					/>
				);
			case "agent":
				return (
					<AgentSection
						draft={draft}
						updateDraft={updateDraft}
						workflow={workflow}
						providerAvailability={providerAvailability}
					/>
				);
			case "background":
				return <BackgroundSection background={background} />;
			case "notifications":
				return <NotificationsSection webhook={webhook} />;
			case "automation":
				return <AutomationSection automation={automation} />;
			case "privacy":
				return <PrivacySection draft={draft} updateDraft={updateDraft} />;
		}
	})();

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="w-[60vw] sm:max-w-5xl h-[70vh] flex flex-col p-0 gap-0 overflow-hidden">
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

					<div className="flex-1 min-h-0 overflow-auto">
						<div className="px-6 py-4">{sectionContent}</div>
					</div>
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
