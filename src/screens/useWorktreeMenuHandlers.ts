import { type MutableRefObject, useCallback, useMemo } from "react";
import type { UseEditorLayoutReturn } from "@/hooks/useEditorLayout";
import { type MenuHandlers, useMenuEvents } from "@/hooks/useMenuEvents";
import type { TabInfo } from "@/types/editor";
import type { AppSettings } from "@/types/settings";
import type {
	EditorAction,
	GitAction,
	UIAction,
	WorktreeGitActions,
} from "./useWorktreeGitActions";

interface UseWorktreeMenuHandlersParams {
	editorLayout: UseEditorLayoutReturn;
	activeTab: TabInfo | null;
	files: TabInfo[];
	saveFile: (path: string) => Promise<void>;
	closeFile: (path: string) => void;
	saveAllDirtyFiles: () => Promise<void>;
	closeAllFiles: () => void;
	createUntitledFile: () => string;
	dispatchEditor: React.Dispatch<EditorAction>;
	dispatchGit: React.Dispatch<GitAction>;
	dispatchUI: React.Dispatch<UIAction>;
	settingsRef: MutableRefObject<AppSettings>;
	onSettingsSaveRef: MutableRefObject<(settings: AppSettings) => void>;
	gitActions: WorktreeGitActions;
	isActive: boolean;
}

export function useWorktreeMenuHandlers({
	editorLayout,
	activeTab,
	files,
	saveFile,
	closeFile,
	saveAllDirtyFiles,
	closeAllFiles,
	createUntitledFile,
	dispatchEditor,
	dispatchGit,
	dispatchUI,
	settingsRef,
	onSettingsSaveRef,
	gitActions,
	isActive,
}: UseWorktreeMenuHandlersParams) {
	const handleSave = useCallback(() => {
		if (!activeTab?.isDirty) return;
		if (activeTab.hasExternalChange) {
			dispatchUI({ type: "SET_SAVING_CONFLICT", path: activeTab.path });
		} else {
			saveFile(activeTab.path);
		}
	}, [activeTab, saveFile, dispatchUI]);

	const handleSearch = useCallback(() => {
		dispatchEditor({ type: "TRIGGER_SEARCH", query: "" });
	}, [dispatchEditor]);

	const handleCloseActiveTab = useCallback(() => {
		if (activeTab) {
			if (activeTab.isDirty) {
				dispatchUI({ type: "SET_CLOSING_TAB", path: activeTab.path });
			} else {
				closeFile(activeTab.path);
				editorLayout.removeTab(activeTab.path);
			}
		}
	}, [activeTab, closeFile, editorLayout, dispatchUI]);

	const handleCreateUntitledTab = useCallback(() => {
		const path = createUntitledFile();
		const name = path.split(":").pop() ?? path;
		editorLayout.addTab(path, name, true);
	}, [createUntitledFile, editorLayout]);

	const menuHandlers: MenuHandlers = useMemo(
		() => ({
			"new-file": handleCreateUntitledTab,
			"new-folder": () => {
				dispatchEditor({ type: "SET_ACTIVE_VIEW", view: "explorer" });
				dispatchEditor({ type: "INCREMENT_NEW_FOLDER" });
			},
			save: handleSave,
			"save-all": saveAllDirtyFiles,
			"close-tab": handleCloseActiveTab,
			"close-all-tabs": () => {
				closeAllFiles();
				for (const file of files) {
					editorLayout.removeTab(file.path);
				}
			},
			"find-in-files": handleSearch,
			"view-explorer": () =>
				dispatchEditor({ type: "SET_ACTIVE_VIEW", view: "explorer" }),
			"view-search": () => {
				dispatchEditor({ type: "TRIGGER_SEARCH", query: "" });
			},
			"view-source-control": () =>
				dispatchEditor({ type: "SET_ACTIVE_VIEW", view: "git" }),
			settings: () => dispatchUI({ type: "SET_SETTINGS_OPEN", open: true }),
			"diff-gutter": () =>
				dispatchGit({ type: "SET_DIFF_MODE", value: "gutter" }),
			"diff-inline": () =>
				dispatchGit({ type: "SET_DIFF_MODE", value: "inline" }),
			"diff-split": () =>
				dispatchGit({ type: "SET_DIFF_MODE", value: "split" }),
			"increase-font-size": () => {
				const s = settingsRef.current;
				onSettingsSaveRef.current({
					...s,
					fontSize: Math.min(24, s.fontSize + 1),
				});
			},
			"decrease-font-size": () => {
				const s = settingsRef.current;
				onSettingsSaveRef.current({
					...s,
					fontSize: Math.max(8, s.fontSize - 1),
				});
			},
			"reset-font-size": () => {
				const s = settingsRef.current;
				onSettingsSaveRef.current({ ...s, fontSize: 14 });
			},
			"git-stage-all": gitActions.handleGitStageAll,
			"git-unstage-all": gitActions.handleGitUnstageAll,
			"git-commit": gitActions.handleGitCommit,
			"git-push": gitActions.handleGitPush,
			"git-discard-all": gitActions.handleGitDiscardAll,
			"git-create-branch": gitActions.handleGitCreateBranch,
			"new-terminal": () => {},
		}),
		[
			handleCreateUntitledTab,
			handleSave,
			saveAllDirtyFiles,
			handleCloseActiveTab,
			closeAllFiles,
			files,
			editorLayout,
			handleSearch,
			dispatchEditor,
			dispatchGit,
			dispatchUI,
			settingsRef,
			onSettingsSaveRef,
			gitActions,
		],
	);

	useMenuEvents(menuHandlers, isActive);

	return { handleSave, handleSearch, handleCloseActiveTab };
}
