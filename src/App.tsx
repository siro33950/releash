import { invoke as invokeTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { DaemonBoundary, useDaemonReady } from "@/components/DaemonBoundary";
import { ProviderHookHealthBanner } from "@/components/layout/ProviderHookHealthBanner";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { UpdateDialog } from "@/components/UpdateDialog";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WorkspaceList } from "@/components/workspace/WorkspaceList";
import { type MenuHandlers, useMenuEvents } from "@/hooks/useMenuEvents";
import { useRepoList } from "@/hooks/useRepoList";
import { useSettings } from "@/hooks/useSettings";
import { useStateSubscriptionResult } from "@/hooks/useStateSubscription";
import { useUpdateChecker } from "@/hooks/useUpdateChecker";
import { useWorkspaceList } from "@/hooks/useWorkspaceList";
import { useWorkspaceNavigation } from "@/hooks/useWorkspaceNavigation";
import { invokeClient } from "@/lib/client";
import { showClientError } from "@/lib/clientErrorNotice";
import { MainLayout } from "@/screens/MainLayout";
import type { CenterSelection } from "@/types/workspace-tree";

type WorktreeCenterState =
	| { phase: "awaitingInitial" }
	| { phase: "selected"; selection: CenterSelection };

function WorkbenchApp() {
	const { settings, updateSettings, updateTheme, loaded, loadError } =
		useSettings();
	const daemonReady = useDaemonReady();
	const updateChecker = useUpdateChecker(settings.autoUpdate);
	const startupStarted = useRef(false);
	const { worktrees, selectedWorktreeId, openWorktreeTab } =
		useWorkspaceNavigation();
	const { repoPaths, addRepo, removeRepo } = useRepoList();
	const workspaceList = useWorkspaceList();
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

	const startupRepository = useStateSubscriptionResult("startup-repository");
	useEffect(() => {
		if (startupRepository.error) showClientError(startupRepository.error);
	}, [startupRepository.error]);
	useEffect(() => {
		if (startupStarted.current || startupRepository.value === undefined) return;
		startupStarted.current = true;
		const worktree = startupRepository.value;
		if (worktree)
			openWorktreeTab(worktree.path, worktree.branch, worktree.repositoryName);
	}, [openWorktreeTab, startupRepository.value]);

	const handleAddRepo = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (!selected) return;
		try {
			const mainPath = await invokeClient("find_repository_root", {
				path: selected,
			});
			if (mainPath) addRepo(mainPath);
			else openWorktreeTab(selected as string);
		} catch (error) {
			showClientError(error);
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
		if (!daemonReady) return;
		invokeTauri("set_menu_items_enabled", { enabled: isWorktreeActive }).catch(
			showClientError,
		);
	}, [isWorktreeActive, daemonReady]);

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
				desktopSettingsLoaded={loaded}
				desktopSettingsError={loadError}
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
				desktopSettingsLoaded={loaded}
				desktopSettingsError={loadError}
				onSave={updateSettings}
				repoPaths={repoPaths ?? []}
				onRemoveRepo={removeRepo}
			/>
		</TooltipProvider>
	);
}

function App() {
	return (
		<DaemonBoundary>
			<WorkbenchApp />
		</DaemonBoundary>
	);
}

export default App;
