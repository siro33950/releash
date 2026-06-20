import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { UpdateDialog } from "@/components/UpdateDialog";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WorkspaceList } from "@/components/workspace/WorkspaceList";
import { type MenuHandlers, useMenuEvents } from "@/hooks/useMenuEvents";
import { useRepoList } from "@/hooks/useRepoList";
import { useSettings } from "@/hooks/useSettings";
import { useUpdateChecker } from "@/hooks/useUpdateChecker";
import { useWorkspaceNavigation } from "@/hooks/useWorkspaceNavigation";
import { setTelemetryEnabled } from "@/lib/telemetry";
import { MainLayout } from "@/screens/MainLayout";
import type { ProviderStatus, WorktreeEntry } from "@/types/git";

function App() {
	const { settings, updateSettings, updateTheme } = useSettings();
	const updateChecker = useUpdateChecker(settings.autoUpdate);
	const { worktrees, selectedWorktreeId, openWorktreeTab } =
		useWorkspaceNavigation();
	const { repoPaths, addRepo, removeRepo, initFromCwd } = useRepoList();

	const [, setInitializing] = useState(true);
	const [, setProviderStatuses] = useState<
		Record<string, ProviderStatus | null>
	>({});
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

export default App;
