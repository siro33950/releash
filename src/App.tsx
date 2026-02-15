import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import { WorkspaceTabBar } from "@/components/layout/WorkspaceTabBar";
import { UpdateDialog } from "@/components/UpdateDialog";
import { type MenuHandlers, useMenuEvents } from "@/hooks/useMenuEvents";
import { useRepoList } from "@/hooks/useRepoList";
import { useSettings } from "@/hooks/useSettings";
import { useUpdateChecker } from "@/hooks/useUpdateChecker";
import { useWorkspaceTabs } from "@/hooks/useWorkspaceTabs";
import { WorkspaceManagerScreen } from "@/screens/WorkspaceManagerScreen";
import { WorktreeView } from "@/screens/WorktreeView";
import type { ProviderStatus, WorktreeEntry } from "@/types/git";

function App() {
	const { settings, updateSettings, updateTheme } = useSettings();
	const updateChecker = useUpdateChecker(settings.autoUpdate);
	const {
		tabs,
		activeTabId,
		openWorktreeTab,
		closeWorktreeTab,
		setActiveTab,
		switchToKanban,
		reorderTabs,
	} = useWorkspaceTabs();
	const { repoPaths, addRepo, removeRepo, initFromCwd } = useRepoList();

	const [initializing, setInitializing] = useState(true);
	const [providerStatuses, setProviderStatuses] = useState<
		Record<string, ProviderStatus | null>
	>({});
	const [kanbanRequestedView, setKanbanRequestedView] = useState<string | null>(
		null,
	);
	const handleKanbanRequestedViewHandled = useCallback(() => {
		setKanbanRequestedView(null);
	}, []);

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
				// git リポジトリ外 → manager のまま
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
			// not a git repo — open as worktree directly
			openWorktreeTab(selected as string);
		}
	}, [addRepo, openWorktreeTab]);

	const handleRemoveRepo = useCallback(
		(repoPath: string) => {
			removeRepo(repoPath);
		},
		[removeRepo],
	);

	// Sync menu item enabled state based on active tab
	const isWorktreeActive = activeTabId !== "kanban";
	useEffect(() => {
		invoke("set_menu_items_enabled", { enabled: isWorktreeActive }).catch(
			() => {},
		);
	}, [isWorktreeActive]);

	const menuHandlers: MenuHandlers = useMemo(
		() => ({
			settings: () => {
				if (activeTabId === "kanban") {
					setKanbanRequestedView("settings");
				}
			},
			"open-folder": handleAddRepo,
			"theme-dark": () => updateTheme("dark"),
			"theme-light": () => updateTheme("light"),
			"back-to-kanban": switchToKanban,
			"remote-start-server": () => {
				switchToKanban();
				setKanbanRequestedView("remote");
			},
			"remote-stop-server": async () => {
				try {
					await invoke("stop_server");
				} catch (e) {
					console.error("Failed to stop server:", e);
				}
			},
			"remote-show-qr": () => {
				switchToKanban();
				setKanbanRequestedView("remote");
			},
		}),
		[activeTabId, handleAddRepo, updateTheme, switchToKanban],
	);

	useMenuEvents(menuHandlers);

	const worktreeTabs = useMemo(
		() => tabs.filter((t) => t.type === "worktree"),
		[tabs],
	);

	return (
		<div className="flex flex-col h-screen w-screen overflow-hidden bg-background text-foreground">
			<UpdateDialog update={updateChecker} />
			<WorkspaceTabBar
				tabs={tabs}
				activeTabId={activeTabId}
				onTabClick={setActiveTab}
				onTabClose={closeWorktreeTab}
				onReorderTabs={reorderTabs}
			/>
			<div className="flex-1 overflow-hidden relative">
				<div
					style={{
						display: activeTabId === "kanban" ? "contents" : "none",
					}}
					className="h-full"
				>
					<WorkspaceManagerScreen
						repoPaths={repoPaths}
						settings={settings}
						providerStatuses={providerStatuses}
						initializing={initializing}
						isActive={activeTabId === "kanban"}
						requestedView={kanbanRequestedView}
						onRequestedViewHandled={handleKanbanRequestedViewHandled}
						onSettingsSave={updateSettings}
						onSelectWorktree={openWorktreeTab}
						onAddRepo={handleAddRepo}
						onRemoveRepo={handleRemoveRepo}
					/>
				</div>
				{worktreeTabs.map((tab) => (
					<div
						key={tab.id}
						style={{
							display: activeTabId === tab.id ? "contents" : "none",
						}}
						className="h-full"
					>
						<WorktreeView
							rootPath={tab.rootPath}
							settings={settings}
							onSettingsSave={updateSettings}
							onSwitchToKanban={switchToKanban}
							isActive={activeTabId === tab.id}
						/>
					</div>
				))}
			</div>
		</div>
	);
}

export default App;
