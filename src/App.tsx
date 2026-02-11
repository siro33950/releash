import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { WorkspaceTabBar } from "@/components/layout/WorkspaceTabBar";
import { useSettings } from "@/hooks/useSettings";
import { useWorkspaceTabs } from "@/hooks/useWorkspaceTabs";
import { WorkspaceManagerScreen } from "@/screens/WorkspaceManagerScreen";
import { WorktreeErrorBoundary } from "@/components/ErrorBoundary";
import { WorktreeView } from "@/screens/WorktreeView";
import type { ProviderStatus, WorktreeEntry } from "@/types/git";

function App() {
	const { settings, updateSettings } = useSettings();
	const {
		tabs,
		activeTabId,
		openWorktreeTab,
		closeWorktreeTab,
		setActiveTab,
		switchToKanban,
	} = useWorkspaceTabs();

	const [mainRepoPath, setMainRepoPath] = useState<string | null>(null);
	const [initializing, setInitializing] = useState(true);
	const [providerStatus, setProviderStatus] = useState<ProviderStatus | null>(
		null,
	);

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
				setMainRepoPath(mainPath);
				const worktrees = await invoke<WorktreeEntry[]>("list_worktrees", {
					repoPath: mainPath,
				});
				if (worktrees.length === 1) {
					openWorktreeTab(worktrees[0].path, worktrees[0].branch);
					setInitializing(false);
					return;
				}
			} catch {
				// git リポジトリ外 → manager のまま
			}
			setInitializing(false);
		})();
	}, [openWorktreeTab]);

	useEffect(() => {
		if (!mainRepoPath) {
			setProviderStatus(null);
			return;
		}
		let cancelled = false;
		invoke<ProviderStatus>("check_pr_provider_status", {
			repoPath: mainRepoPath,
		})
			.then((s) => {
				if (!cancelled) setProviderStatus(s);
			})
			.catch(() => {
				if (!cancelled) setProviderStatus(null);
			});
		return () => {
			cancelled = true;
		};
	}, [mainRepoPath]);

	const handleChangeRepo = useCallback((path: string | null) => {
		setMainRepoPath(path);
	}, []);

	const worktreeTabs = useMemo(
		() => tabs.filter((t) => t.type === "worktree"),
		[tabs],
	);

	return (
		<div className="flex flex-col h-screen w-screen overflow-hidden bg-background text-foreground">
			<WorkspaceTabBar
				tabs={tabs}
				activeTabId={activeTabId}
				onTabClick={setActiveTab}
				onTabClose={closeWorktreeTab}
			/>
			<div className="flex-1 overflow-hidden relative">
				<div
					style={{
						display: activeTabId === "kanban" ? "contents" : "none",
					}}
					className="h-full"
				>
					<WorkspaceManagerScreen
						repoPath={mainRepoPath}
						settings={settings}
						providerStatus={providerStatus}
						initializing={initializing}
						onSettingsSave={updateSettings}
						onSelectWorktree={openWorktreeTab}
						onChangeRepo={handleChangeRepo}
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
						<WorktreeErrorBoundary>
							<WorktreeView
								rootPath={tab.rootPath}
								settings={settings}
								onSettingsSave={updateSettings}
								onSwitchToKanban={switchToKanban}
							/>
						</WorktreeErrorBoundary>
					</div>
				))}
			</div>
		</div>
	);
}

export default App;
