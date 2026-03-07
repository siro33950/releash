import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import { RemotePanel } from "@/components/panels/RemotePanel";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { UpdateDialog } from "@/components/UpdateDialog";
import {
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WorkspaceList } from "@/components/workspace/WorkspaceList";
import { useBatchSpawnAgents } from "@/hooks/useBatchSpawnAgents";
import { type MenuHandlers, useMenuEvents } from "@/hooks/useMenuEvents";
import { useRemoteAutoStart } from "@/hooks/useRemoteAutoStart";
import { useRepoList } from "@/hooks/useRepoList";
import { useSettings } from "@/hooks/useSettings";
import { useUpdateChecker } from "@/hooks/useUpdateChecker";
import { useWorkspaceNavigation } from "@/hooks/useWorkspaceNavigation";
import { setTelemetryEnabled } from "@/lib/telemetry";
import { MainLayout } from "@/screens/MainLayout";
import type { ProviderStatus, WorktreeEntry } from "@/types/git";
import { buildTerminalCommand } from "@/types/settings";

function App() {
	const { settings, updateSettings, updateTheme } = useSettings();
	const updateChecker = useUpdateChecker(settings.autoUpdate);
	const { worktrees, selectedWorktreeId, openWorktreeTab } =
		useWorkspaceNavigation();
	const { repoPaths, addRepo, initFromCwd } = useRepoList();

	const [initializing, setInitializing] = useState(true);
	useRemoteAutoStart(!initializing);
	const [, setProviderStatuses] = useState<
		Record<string, ProviderStatus | null>
	>({});
	const [showRemote, setShowRemote] = useState(false);
	const [showAppSettings, setShowAppSettings] = useState(false);

	useEffect(() => {
		setTelemetryEnabled(settings.telemetryEnabled);
	}, [settings.telemetryEnabled]);

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
					setInitializing(false);
					return;
				}
			} catch {
				// git リポジトリ外
			}
			setInitializing(false);
		})();
	}, [openWorktreeTab, initFromCwd]);

	useEffect(() => {
		if (repoPaths.length === 0) {
			setProviderStatuses({});
			return;
		}
		let cancelled = false;
		const fetchStatuses = async () => {
			const entries = await Promise.all(
				repoPaths.map(async (repoPath) => {
					try {
						const status = await invoke<ProviderStatus>(
							"check_pr_provider_status",
							{ repoPath },
						);
						return [repoPath, status] as const;
					} catch {
						return [repoPath, null] as const;
					}
				}),
			);
			if (!cancelled) {
				setProviderStatuses(Object.fromEntries(entries));
			}
		};
		fetchStatuses();
		return () => {
			cancelled = true;
		};
	}, [repoPaths]);

	useBatchSpawnAgents(
		repoPaths,
		settings.agent,
		buildTerminalCommand(settings),
		settings.agentMaxConcurrent,
	);

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
		(rootPath: string, branchName?: string, repoName?: string) => {
			openWorktreeTab(rootPath, branchName, repoName);
		},
		[openWorktreeTab],
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
			"remote-start-server": () => setShowRemote(true),
			"remote-stop-server": async () => {
				try {
					await invoke("stop_server");
				} catch (e) {
					console.error("Failed to stop server:", e);
				}
			},
			"remote-show-qr": () => setShowRemote(true),
		}),
		[handleAddRepo, updateTheme],
	);

	useMenuEvents(menuHandlers);

	const selectedRootPath = useMemo(() => {
		if (!selectedWorktreeId) return null;
		const tab = worktrees.find((t) => t.id === selectedWorktreeId);
		return tab?.rootPath ?? null;
	}, [worktrees, selectedWorktreeId]);

	const leftNav = useMemo(
		() => (
			<WorkspaceList
				repoPaths={repoPaths}
				selectedRootPath={selectedRootPath}
				onSelectWorktree={handleSelectWorktree}
				onAddRepo={handleAddRepo}
				onShowRemote={() => setShowRemote(true)}
				onShowSettings={() => setShowAppSettings(true)}
			/>
		),
		[repoPaths, selectedRootPath, handleSelectWorktree, handleAddRepo],
	);

	return (
		<TooltipProvider>
			<UpdateDialog update={updateChecker} />
			<MainLayout
				selectedRootPath={selectedRootPath}
				settings={settings}
				onSettingsSave={updateSettings}
				leftNav={leftNav}
			/>

			{/* Remote Dialog */}
			<Dialog open={showRemote} onOpenChange={setShowRemote}>
				<DialogContent className="max-w-lg">
					<DialogHeader>
						<DialogTitle>Remote Access</DialogTitle>
					</DialogHeader>
					<RemotePanel
						terminalStartupCommand={buildTerminalCommand(settings)}
					/>
				</DialogContent>
			</Dialog>

			{/* App Settings */}
			<SettingsModal
				open={showAppSettings}
				onOpenChange={setShowAppSettings}
				settings={settings}
				onSave={updateSettings}
				repoPaths={repoPaths}
			/>
		</TooltipProvider>
	);
}

export default App;
