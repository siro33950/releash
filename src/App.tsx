import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ApplicationShutdownBanner } from "@/components/layout/ApplicationShutdownBanner";
import { ProviderHookHealthBanner } from "@/components/layout/ProviderHookHealthBanner";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { UpdateDialog } from "@/components/UpdateDialog";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WorkspaceList } from "@/components/workspace/WorkspaceList";
import { type MenuHandlers, useMenuEvents } from "@/hooks/useMenuEvents";
import { useRepoList } from "@/hooks/useRepoList";
import { useSettings } from "@/hooks/useSettings";
import { useUpdateChecker } from "@/hooks/useUpdateChecker";
import { useWorkspaceNavigation } from "@/hooks/useWorkspaceNavigation";
import { MainLayout } from "@/screens/MainLayout";
import type { WorktreeEntry } from "@/types/git";
import type {
	CenterSelection,
	NewSessionCreationRequest,
	NewSessionCreationStatus,
} from "@/types/workspace-tree";

type WorktreeCenterState =
	| { phase: "awaitingInitial" }
	| { phase: "selected"; selection: CenterSelection };

interface NewSessionCreationState {
	request: NewSessionCreationRequest;
	status: "pending" | "failed";
	error: string | null;
}

type StartupFailureKind =
	| "store_in_use"
	| "storage_unavailable"
	| "unsupported_runtime"
	| "unsupported_store_version"
	| "initialization_state_invalid"
	| "store_validation_failed"
	| "schema_evolution_failed";

type ApplicationStartupOutcome =
	| { type: "ready" }
	| {
			type: "failed";
			kind: StartupFailureKind;
			safeDescription: string;
			correlationId: string;
			retryOnNextLaunch: boolean;
			actions: ["quit"];
	  };

function StartupFailureScreen({
	failure,
}: {
	failure: Extract<ApplicationStartupOutcome, { type: "failed" }>;
}) {
	const [quitting, setQuitting] = useState(false);
	const quit = useCallback(async () => {
		if (quitting) return;
		setQuitting(true);
		try {
			await invoke("quit_after_startup_failure");
		} catch {
			setQuitting(false);
		}
	}, [quitting]);
	return (
		<main className="flex min-h-screen items-center justify-center bg-background p-6 text-foreground">
			<section
				aria-labelledby="startup-failure-title"
				className="w-full max-w-lg rounded-xl border border-border bg-card p-6 shadow-lg"
			>
				<p className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
					Releash could not start
				</p>
				<h1 id="startup-failure-title" className="text-xl font-semibold">
					{failure.safeDescription}
				</h1>
				<p className="mt-3 text-sm text-muted-foreground">
					Classification:{" "}
					<code className="font-mono text-xs">{failure.kind}</code>
				</p>
				<p className="mt-4 text-sm">
					{failure.retryOnNextLaunch
						? "Quit Releash, then launch it again to retry."
						: "Quit Releash and use a compatible build or resolve the local data issue before launching again."}
				</p>
				<p className="mt-4 break-all font-mono text-xs text-muted-foreground">
					Correlation: {failure.correlationId}
				</p>
				<button
					type="button"
					className="mt-6 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground disabled:opacity-50"
					disabled={quitting}
					onClick={() => void quit()}
				>
					{quitting ? "Quitting…" : "Quit"}
				</button>
			</section>
		</main>
	);
}

function WorkbenchApp() {
	const { settings, updateSettings, updateTheme } = useSettings();
	const updateChecker = useUpdateChecker(settings.autoUpdate);
	const { worktrees, selectedWorktreeId, openWorktreeTab } =
		useWorkspaceNavigation();
	const { repoPaths, addRepo, removeRepo, initFromCwd } = useRepoList();

	const [showAppSettings, setShowAppSettings] = useState(false);
	const [centerStateByWorktree, setCenterStateByWorktree] = useState<
		Record<string, WorktreeCenterState>
	>({});
	const [newSessionCreationByWorktree, setNewSessionCreationByWorktree] =
		useState<Record<string, NewSessionCreationState>>({});

	const selectedRootPath = useMemo(() => {
		if (!selectedWorktreeId) return null;
		const tab = worktrees.find((t) => t.id === selectedWorktreeId);
		return tab?.rootPath ?? null;
	}, [worktrees, selectedWorktreeId]);
	const activeCenterState = selectedRootPath
		? (centerStateByWorktree[selectedRootPath] ?? {
				phase: "awaitingInitial" as const,
			})
		: null;
	const centerSelection =
		activeCenterState?.phase === "selected"
			? activeCenterState.selection
			: null;
	const centerSelectionByWorktree = useMemo(() => {
		const map: Record<string, CenterSelection | null> = {};
		for (const [rootPath, state] of Object.entries(centerStateByWorktree)) {
			map[rootPath] = state.phase === "selected" ? state.selection : null;
		}
		return map;
	}, [centerStateByWorktree]);
	const activeNewSessionCreation = selectedRootPath
		? newSessionCreationByWorktree[selectedRootPath]
		: undefined;
	const newSessionCreationRequest =
		activeNewSessionCreation?.status === "pending"
			? activeNewSessionCreation.request
			: null;
	const newSessionCreationStatusByWorktree = useMemo<
		Record<string, NewSessionCreationStatus>
	>(() => {
		return Object.fromEntries(
			Object.entries(newSessionCreationByWorktree).map(
				([worktreePath, state]) => [
					worktreePath,
					{ pending: state.status === "pending", error: state.error },
				],
			),
		);
	}, [newSessionCreationByWorktree]);

	useEffect(() => {
		const suppress = (e: MouseEvent) => e.preventDefault();
		document.addEventListener("contextmenu", suppress);
		return () => document.removeEventListener("contextmenu", suppress);
	}, []);

	useEffect(() => {
		(async () => {
			try {
				const cwd = await invoke<string>("get_cwd");
				const mainPath = await invoke<string>("get_main_repo_path", {
					anyPath: cwd,
				});
				initFromCwd(mainPath);
				const worktrees = await invoke<WorktreeEntry[]>("list_worktrees", {
					repoPath: mainPath,
				});
				if (worktrees.length === 1) {
					const repoName = mainPath.split(/[\\/]/).pop() ?? mainPath;
					openWorktreeTab(worktrees[0].path, worktrees[0].branch, repoName);
				}
			} catch {
				// git リポジトリ外
			}
		})();
	}, [openWorktreeTab, initFromCwd]);

	const handleAddRepo = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (!selected) return;
		try {
			const mainPath = await invoke<string>("get_main_repo_path", {
				anyPath: selected,
			});
			addRepo(mainPath);
		} catch {
			openWorktreeTab(selected as string);
		}
	}, [addRepo, openWorktreeTab]);

	const handleSelectWorktree = useCallback(
		(
			rootPath: string,
			branchName?: string,
			repoName?: string,
			centerSelection?: CenterSelection,
		) => {
			openWorktreeTab(rootPath, branchName, repoName);
			if (centerSelection) {
				setCenterStateByWorktree((current) => ({
					...current,
					[rootPath]: { phase: "selected", selection: centerSelection },
				}));
			}
		},
		[openWorktreeTab],
	);
	const handleCreateSession = useCallback(
		(rootPath: string, branchName?: string, repoName?: string) => {
			openWorktreeTab(rootPath, branchName, repoName);
			setNewSessionCreationByWorktree((current) => {
				const existing = current[rootPath];
				if (existing?.status === "pending") return current;
				const request: NewSessionCreationRequest = {
					requestId: globalThis.crypto.randomUUID(),
					worktreePath: rootPath,
				};
				return {
					...current,
					[rootPath]: { request, status: "pending", error: null },
				};
			});
		},
		[openWorktreeTab],
	);

	const handleNewSessionCreated = useCallback(
		(request: NewSessionCreationRequest, selection: CenterSelection) => {
			setCenterStateByWorktree((current) => ({
				...current,
				[selection.worktreePath]: { phase: "selected", selection },
			}));
			setNewSessionCreationByWorktree((current) => {
				const active = current[request.worktreePath];
				if (active?.request.requestId !== request.requestId) {
					return current;
				}
				const next = { ...current };
				delete next[request.worktreePath];
				return next;
			});
		},
		[],
	);
	const handleNewSessionCreationFailed = useCallback(
		(request: NewSessionCreationRequest, error: string) => {
			setNewSessionCreationByWorktree((current) => {
				const active = current[request.worktreePath];
				if (active?.request.requestId !== request.requestId) {
					return current;
				}
				return {
					...current,
					[request.worktreePath]: {
						...active,
						status: "failed",
						error: `Session creation failed: ${error}`,
					},
				};
			});
		},
		[],
	);
	const handleCenterSelectionInvalidated = useCallback(
		(worktreePath: string, nodeId: string) => {
			setCenterStateByWorktree((current) => {
				const active = current[worktreePath];
				if (
					active?.phase !== "selected" ||
					active.selection.kind === "provider_agent_session_launching" ||
					(active.selection.kind === "node"
						? active.selection.nodeId
						: active.selection.agentSessionId) !== nodeId
				) {
					return current;
				}
				return {
					...current,
					[worktreePath]: { phase: "awaitingInitial" },
				};
			});
		},
		[],
	);
	const handleProviderAgentSessionLaunchConsumed = useCallback(
		(agentSessionId: string) => {
			setCenterStateByWorktree((current) => {
				for (const [worktreePath, state] of Object.entries(current)) {
					if (
						state.phase !== "selected" ||
						state.selection.kind !== "provider_agent_session" ||
						state.selection.agentSessionId !== agentSessionId ||
						!state.selection.initialAttachment
					) {
						continue;
					}
					const selection: CenterSelection = {
						kind: "provider_agent_session",
						worktreePath: state.selection.worktreePath,
						agentSessionId: state.selection.agentSessionId,
					};
					return {
						...current,
						[worktreePath]: { phase: "selected", selection },
					};
				}
				return current;
			});
		},
		[],
	);
	const isWorktreeActive = selectedWorktreeId != null;
	useEffect(() => {
		invoke("set_menu_items_enabled", { enabled: isWorktreeActive }).catch(
			() => {},
		);
	}, [isWorktreeActive]);

	const menuHandlers: MenuHandlers = useMemo(
		() => ({
			settings: () => setShowAppSettings(true),
			"open-folder": handleAddRepo,
			"theme-dark": () => updateTheme("dark"),
			"theme-light": () => updateTheme("light"),
			"back-to-kanban": () => {},
		}),
		[handleAddRepo, updateTheme],
	);

	useMenuEvents(menuHandlers);

	const leftNav = useMemo(
		() => (
			<WorkspaceList
				repoPaths={repoPaths}
				selectedRootPath={selectedRootPath}
				centerSelection={centerSelection}
				autoSelectPreferredNode={activeCenterState?.phase === "awaitingInitial"}
				newSessionCreationStatusByWorktree={newSessionCreationStatusByWorktree}
				onSelectWorktree={handleSelectWorktree}
				onCreateSession={handleCreateSession}
				onWorkspaceSelectionInvalidated={handleCenterSelectionInvalidated}
				onAddRepo={handleAddRepo}
				onShowSettings={() => setShowAppSettings(true)}
			/>
		),
		[
			repoPaths,
			selectedRootPath,
			centerSelection,
			activeCenterState?.phase,
			newSessionCreationStatusByWorktree,
			handleSelectWorktree,
			handleCreateSession,
			handleCenterSelectionInvalidated,
			handleAddRepo,
		],
	);

	return (
		<TooltipProvider>
			<UpdateDialog update={updateChecker} />
			<MainLayout
				selectedRootPath={selectedRootPath}
				settings={settings}
				onSettingsSave={updateSettings}
				leftNav={leftNav}
				topBanner={
					<>
						<ApplicationShutdownBanner />
						<ProviderHookHealthBanner />
					</>
				}
				centerSelectionByWorktree={centerSelectionByWorktree}
				newSessionCreationRequest={newSessionCreationRequest}
				onNewSessionCreated={handleNewSessionCreated}
				onNewSessionCreationFailed={handleNewSessionCreationFailed}
				onProviderAgentSessionLaunchConsumed={
					handleProviderAgentSessionLaunchConsumed
				}
				onCenterNodeMissing={handleCenterSelectionInvalidated}
			/>

			{/* App Settings */}
			<SettingsModal
				open={showAppSettings}
				onOpenChange={setShowAppSettings}
				settings={settings}
				onSave={updateSettings}
				repoPaths={repoPaths}
				onRemoveRepo={removeRepo}
			/>
		</TooltipProvider>
	);
}

function App() {
	const [outcome, setOutcome] = useState<ApplicationStartupOutcome | null>(
		null,
	);
	const [outcomeUnavailable, setOutcomeUnavailable] = useState(false);

	useEffect(() => {
		let active = true;
		void invoke<ApplicationStartupOutcome>("get_application_startup_outcome")
			.then((result) => {
				if (active) setOutcome(result);
			})
			.catch(() => {
				if (active) setOutcomeUnavailable(true);
			});
		return () => {
			active = false;
		};
	}, []);

	if (outcomeUnavailable) {
		return (
			<main className="flex min-h-screen items-center justify-center bg-background p-6 text-foreground">
				<section className="w-full max-w-lg rounded-xl border border-border bg-card p-6 shadow-lg">
					<p className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
						Releash could not start
					</p>
					<h1 className="text-xl font-semibold">Startup outcome unavailable</h1>
					<p className="mt-3 text-sm leading-6 text-muted-foreground">
						Close Releash and launch it again. No application operation is
						available in this state.
					</p>
				</section>
			</main>
		);
	}
	if (!outcome) {
		return (
			<main
				aria-label="Starting Releash"
				className="flex min-h-screen items-center justify-center bg-background text-sm text-muted-foreground"
			>
				Starting Releash…
			</main>
		);
	}
	if (outcome.type === "failed") {
		return <StartupFailureScreen failure={outcome} />;
	}
	return <WorkbenchApp />;
}

export default App;
