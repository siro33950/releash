import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useSettings } from "@/hooks/useSettings";
import { WorkspaceManagerScreen } from "@/screens/WorkspaceManagerScreen";
import { WorktreeView } from "@/screens/WorktreeView";
import type { ProviderStatus, WorktreeEntry } from "@/types/git";
import type { ScreenType } from "@/types/screen";

function App() {
	const {
		settings,
		updateTheme,
		updateFontSize,
		updateDefaultDiffBase,
		updateDefaultDiffMode,
		updateTerminalStartupCommand,
	} = useSettings();

	const [screen, setScreen] = useState<ScreenType>("manager");
	const [rootPath, setRootPath] = useState<string | null>(null);
	const [mainRepoPath, setMainRepoPath] = useState<string | null>(null);
	const [initializing, setInitializing] = useState(true);
	const [providerStatus, setProviderStatus] =
		useState<ProviderStatus | null>(null);

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
					setRootPath(worktrees[0].path);
					setScreen("workspace");
					setInitializing(false);
					return;
				}
			} catch {
				// git リポジトリ外 → manager のまま
			}
			setInitializing(false);
		})();
	}, []);

	useEffect(() => {
		if (!mainRepoPath) {
			setProviderStatus(null);
			return;
		}
		invoke<ProviderStatus>("check_pr_provider_status", {
			repoPath: mainRepoPath,
		})
			.then(setProviderStatus)
			.catch(() => setProviderStatus(null));
	}, [mainRepoPath]);

	const handleChangeRepo = useCallback((path: string | null) => {
		setMainRepoPath(path);
	}, []);

	const handleGoHome = useCallback(() => {
		setScreen("manager");
		setRootPath(null);
	}, []);

	const handleSelectWorktree = useCallback((path: string) => {
		setRootPath(path);
		setScreen("workspace");
	}, []);

	const showWorkspace = screen === "workspace" && !!rootPath;

	return showWorkspace ? (
		<WorktreeView
			key={rootPath}
			rootPath={rootPath}
			settings={settings}
			updateTheme={updateTheme}
			updateFontSize={updateFontSize}
			updateDefaultDiffBase={updateDefaultDiffBase}
			updateDefaultDiffMode={updateDefaultDiffMode}
			updateTerminalStartupCommand={updateTerminalStartupCommand}
			onGoHome={handleGoHome}
		/>
	) : (
		<WorkspaceManagerScreen
			repoPath={mainRepoPath}
			settings={settings}
			providerStatus={providerStatus}
			initializing={initializing}
			onThemeChange={updateTheme}
			onFontSizeChange={updateFontSize}
			onDiffBaseChange={updateDefaultDiffBase}
			onDiffModeChange={updateDefaultDiffMode}
			onTerminalStartupCommandChange={updateTerminalStartupCommand}
			onSelectWorktree={handleSelectWorktree}
			onChangeRepo={handleChangeRepo}
		/>
	);
}

export default App;
