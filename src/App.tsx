import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useSettings } from "@/hooks/useSettings";
import { WorkspaceManagerScreen } from "@/screens/WorkspaceManagerScreen";
import { WorktreeView } from "@/screens/WorktreeView";
import type { WorktreeEntry } from "@/types/git";
import type { ScreenType } from "@/types/screen";

function App() {
	const {
		settings,
		updateTheme,
		updateFontSize,
		updateDefaultDiffBase,
		updateDefaultDiffMode,
	} = useSettings();

	const [screen, setScreen] = useState<ScreenType>("manager");
	const [rootPath, setRootPath] = useState<string | null>(null);
	const [mainRepoPath, setMainRepoPath] = useState<string | null>(null);

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
				}
			} catch {
				// git リポジトリ外 → manager のまま
			}
		})();
	}, []);

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

	if (screen === "workspace" && rootPath) {
		return (
			<WorktreeView
				key={rootPath}
				rootPath={rootPath}
				settings={settings}
				updateTheme={updateTheme}
				updateFontSize={updateFontSize}
				updateDefaultDiffBase={updateDefaultDiffBase}
				updateDefaultDiffMode={updateDefaultDiffMode}
				onGoHome={handleGoHome}
			/>
		);
	}

	return (
		<WorkspaceManagerScreen
			repoPath={mainRepoPath}
			onSelectWorktree={handleSelectWorktree}
			onChangeRepo={handleChangeRepo}
		/>
	);
}

export default App;
