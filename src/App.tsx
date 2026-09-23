import { invoke as invokeTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	DaemonBoundary,
	useDesktopRestoration,
} from "@/components/DaemonBoundary";
import { ProviderHookHealthBanner } from "@/components/layout/ProviderHookHealthBanner";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { UpdateDialog } from "@/components/UpdateDialog";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WorkspaceList } from "@/components/workspace/WorkspaceList";
import { type MenuHandlers, useMenuEvents } from "@/hooks/useMenuEvents";
import { useRepoList } from "@/hooks/useRepoList";
import { useSettings } from "@/hooks/useSettings";
import { useUpdateChecker } from "@/hooks/useUpdateChecker";
import { useWorkspaceList } from "@/hooks/useWorkspaceList";
import { useWorkspaceNavigation } from "@/hooks/useWorkspaceNavigation";
import { invokeClient as invoke } from "@/lib/client";
import { MainLayout } from "@/screens/MainLayout";
import type { CenterSelection } from "@/types/workspace-tree";

type WorktreeCenterState =
	| { phase: "awaitingInitial" }
	| { phase: "selected"; selection: CenterSelection };

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
			await invokeTauri("quit_after_startup_failure");
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
	const {
		settings,
		updateSettings,
		updateTheme,
		loaded: settingsLoaded,
		loadError: settingsError,
	} = useSettings();
	const restoration = useDesktopRestoration();
	const updateChecker = useUpdateChecker(
		settings.autoUpdate && restoration.ready,
	);
	const { worktrees, selectedWorktreeId, openWorktreeTab } =
		useWorkspaceNavigation();
	const { addRepo, removeRepo, initFromCwd } = useRepoList();
	const workspaceList = useWorkspaceList();
	const repoPaths =
		workspaceList.snapshot?.repositories.map((repo) => repo.path) ?? [];
	const repositoriesLoaded = workspaceList.snapshot?.status.loaded;
	const repositoriesError =
		workspaceList.error ?? workspaceList.snapshot?.status.error;

	useEffect(() => {
		if (
			!restoration.ready &&
			settingsLoaded &&
			(repositoriesLoaded || repositoriesError)
		)
			void restoration.complete();
	}, [restoration, settingsLoaded, repositoriesLoaded, repositoriesError]);
	useEffect(() => {
		if (settingsError) void restoration.fail(`Settings: ${settingsError}`);
	}, [restoration, settingsError]);

	const [showAppSettings, setShowAppSettings] = useState(false);
	const [centerStateByWorktree, setCenterStateByWorktree] = useState<
		Record<string, WorktreeCenterState>
	>({});
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
	useEffect(() => {
		const suppress = (e: MouseEvent) => e.preventDefault();
		document.addEventListener("contextmenu", suppress);
		return () => document.removeEventListener("contextmenu", suppress);
	}, []);

	useEffect(() => {
		if (!restoration.ready) return;
		(async () => {
			try {
				const cwd = await invoke("get_cwd");
				const mainPath = await invoke("get_main_repo_path", {
					anyPath: cwd,
				});
				initFromCwd(mainPath);
				const worktrees = await invoke("list_worktrees", {
					repoPath: mainPath,
				});
				const workingAreas = worktrees;
				if (workingAreas.length === 1) {
					const repoName = mainPath.split(/[\\/]/).pop() ?? mainPath;
					openWorktreeTab(
						workingAreas[0].path,
						workingAreas[0].branch,
						repoName,
					);
				}
			} catch {
				// git リポジトリ外
			}
		})();
	}, [openWorktreeTab, initFromCwd, restoration.ready]);

	const handleAddRepo = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (!selected) return;
		try {
			const mainPath = await invoke("get_main_repo_path", {
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
	const handleCenterSelectionInvalidated = useCallback(
		(worktreePath: string, nodeId: string) => {
			setCenterStateByWorktree((current) => {
				const active = current[worktreePath];
				if (
					active?.phase !== "selected" ||
					active.selection.kind === "agent_session_launching" ||
					active.selection.nodeId !== nodeId
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
	const handleCenterSessionAttachmentConsumed = useCallback(
		(worktreePath: string, nodeId: string, agentSessionId: string) => {
			setCenterStateByWorktree((current) => {
				const active = current[worktreePath];
				if (
					active?.phase !== "selected" ||
					active.selection.kind !== "node" ||
					active.selection.nodeId !== nodeId ||
					active.selection.initialSessionAttachment?.agentSessionId !==
						agentSessionId
				) {
					return current;
				}
				return {
					...current,
					[worktreePath]: {
						phase: "selected",
						selection: {
							kind: "node",
							worktreePath,
							nodeId,
						},
					},
				};
			});
		},
		[],
	);
	const isWorktreeActive = selectedWorktreeId != null;
	useEffect(() => {
		invokeTauri("set_menu_items_enabled", { enabled: isWorktreeActive }).catch(
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
				model={workspaceList}
				selectedRootPath={selectedRootPath}
				centerSelection={centerSelection}
				autoSelectPreferredNode={activeCenterState?.phase === "awaitingInitial"}
				onSelectWorktree={handleSelectWorktree}
				onWorkspaceSelectionInvalidated={handleCenterSelectionInvalidated}
				onAddRepo={handleAddRepo}
				onShowSettings={() => setShowAppSettings(true)}
			/>
		),
		[
			workspaceList,
			selectedRootPath,
			centerSelection,
			activeCenterState?.phase,
			handleSelectWorktree,
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
				topBanner={<ProviderHookHealthBanner />}
				centerSelectionByWorktree={centerSelectionByWorktree}
				onCenterNodeMissing={handleCenterSelectionInvalidated}
				onCenterSessionAttachmentConsumed={
					handleCenterSessionAttachmentConsumed
				}
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
		void invokeTauri<ApplicationStartupOutcome>(
			"get_application_startup_outcome",
		)
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
	return (
		<DaemonBoundary>
			<WorkbenchApp />
		</DaemonBoundary>
	);
}

export default App;
