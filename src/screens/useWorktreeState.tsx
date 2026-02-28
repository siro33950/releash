import { listen } from "@tauri-apps/api/event";
import {
	useCallback,
	useEffect,
	useMemo,
	useReducer,
	useRef,
	useState,
} from "react";
import { SidebarPanel } from "@/components/panels/SidebarPanel";
import type { TerminalTabPanelHandle } from "@/components/panels/TerminalTabPanel";
import type { EditorContextValue } from "@/contexts/EditorContext";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useEditorLayout } from "@/hooks/useEditorLayout";
import { useFileContents } from "@/hooks/useFileContents";
import { type FileChangeEvent, useFileWatcher } from "@/hooks/useFileWatcher";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitDirWatcher } from "@/hooks/useGitDirWatcher";
import { useLineComments } from "@/hooks/useLineComments";
import { useNativeFileDrop } from "@/hooks/useNativeFileDrop";
import { agentStateKey, aggregateAgentState } from "@/lib/agentStateUtils";
import { registerDefinitionProviders } from "@/lib/monaco-definition-provider";
import { normalizePath } from "@/lib/normalizePath";
import { useWorktreeComments } from "@/screens/useWorktreeComments";
import {
	editorReducer,
	gitReducer,
	initialEditorState,
	initialUIState,
	uiReducer,
	useWorktreeGitActions,
} from "@/screens/useWorktreeGitActions";
import { useWorktreeMenuHandlers } from "@/screens/useWorktreeMenuHandlers";
import type { AgentStateSync } from "@/types/protocol";
import type { AppSettings, DiffBase, DiffMode } from "@/types/settings";

interface UseWorktreeStateParams {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	isActive: boolean;
	centerTabRef?: React.RefObject<string>;
}

export function useWorktreeState({
	rootPath,
	settings,
	onSettingsSave,
	isActive,
	centerTabRef,
}: UseWorktreeStateParams) {
	const {
		files,
		getFileContent,
		openFile,
		closeFile,
		updateContent,
		saveFile,
		reloadFileIfClean,
		markExternalChange,
		clearExternalChange,
		updateFilePath,
		closeFilesByPrefix,
		closeAllFiles,
		saveAllDirtyFiles,
		createUntitledFile,
	} = useFileContents();

	const [editor, dispatchEditor] = useReducer(
		editorReducer,
		initialEditorState,
	);
	const {
		activeView,
		searchFocusKey,
		searchInitialQuery,
		pendingReveal,
		newFolderKey,
	} = editor;

	const { branch } = useCurrentBranch(rootPath);
	const [ready, setReady] = useState(false);
	const [agentStatesMap, setAgentStatesMap] = useState<
		Map<string, AgentStateSync>
	>(new Map());
	const {
		comments,
		addComment,
		removeComment,
		updateComment,
		markAsSent,
		resolveComment,
		showSentComments,
		toggleShowSentComments,
		showInlineComments,
		toggleShowInlineComments,
	} = useLineComments(rootPath);
	const { stage, unstage, push, discard, stageHunk, createBranch } =
		useGitActions();
	const terminalRef = useRef<TerminalTabPanelHandle>(null);

	const [git, dispatchGit] = useReducer(gitReducer, {
		diffBase: settings.defaultDiffBase,
		diffMode: settings.defaultDiffMode,
		gitError: null,
		refreshKey: 0,
	});
	const { diffBase, diffMode, gitError, refreshKey: gitRefreshKey } = git;

	const refreshGit = useCallback(() => dispatchGit({ type: "REFRESH" }), []);
	const setDiffBase = useCallback(
		(value: DiffBase) => dispatchGit({ type: "SET_DIFF_BASE", value }),
		[],
	);
	const setDiffMode = useCallback(
		(value: DiffMode) => dispatchGit({ type: "SET_DIFF_MODE", value }),
		[],
	);

	const [ui, dispatchUI] = useReducer(uiReducer, initialUIState);
	const {
		isSettingsOpen,
		closingTabPath,
		savingConflictPath,
		showDiscardConfirm,
		showCreateBranch,
		newBranchName,
		editorDragOver,
	} = ui;

	const handleTabClose = useCallback(
		(path: string): boolean => {
			const file = getFileContent(path);
			if (file?.isDirty) {
				dispatchUI({ type: "SET_CLOSING_TAB", path });
				return true;
			}
			closeFile(path);
			return false;
		},
		[getFileContent, closeFile],
	);

	const editorLayout = useEditorLayout(handleTabClose);
	const activeTabPath = editorLayout.getActiveTabPath();
	const activeTab = activeTabPath ? getFileContent(activeTabPath) : null;

	// --- File watcher ---
	useFileWatcher({
		rootPath,
		onFileChange: useCallback(
			(event: FileChangeEvent) => {
				const path = normalizePath(event.path);
				const file = getFileContent(path);
				if (file?.isDirty) {
					markExternalChange(path);
				} else {
					reloadFileIfClean(path);
				}
			},
			[reloadFileIfClean, getFileContent, markExternalChange],
		),
	});

	// --- Git dir watcher (index / refs / HEAD) ---
	useGitDirWatcher(rootPath);

	// --- Lifecycle effects ---
	useEffect(() => {
		if (activeView === "git") refreshGit();
	}, [activeView, refreshGit]);

	useEffect(() => {
		if (branch != null) setReady(true);
	}, [branch]);

	useEffect(() => {
		const unlisten = listen<AgentStateSync>("agent-state-changed", (event) => {
			if (
				normalizePath(event.payload.worktree_path) === normalizePath(rootPath)
			) {
				setAgentStatesMap((prev) => {
					const key = agentStateKey(
						event.payload.worktree_path,
						event.payload.pty_id,
					);
					const next = new Map(prev);
					next.set(key, event.payload);
					return next;
				});
			}
		});
		return () => {
			unlisten.then((f) => f());
		};
	}, [rootPath]);

	const agentState = useMemo(
		() => aggregateAgentState(agentStatesMap, rootPath),
		[agentStatesMap, rootPath],
	);

	// --- Refs ---
	const rootPathRef = useRef(rootPath);
	rootPathRef.current = rootPath;

	const commentsRef = useRef(comments);
	commentsRef.current = comments;

	const settingsRef = useRef(settings);
	settingsRef.current = settings;

	const onSettingsSaveRef = useRef(onSettingsSave);
	onSettingsSaveRef.current = onSettingsSave;

	// --- File open sync ---
	const handleOpenFile = useCallback(
		async (path: string) => {
			await openFile(path);
			const file = getFileContent(path);
			const name = path.split(/[/\\]/).pop() ?? path;
			editorLayout.addTab(path, name, file?.isDirty ?? false);
		},
		[openFile, getFileContent, editorLayout],
	);
	const handleOpenFileRef = useRef(handleOpenFile);
	handleOpenFileRef.current = handleOpenFile;

	const editorLayoutRef = useRef(editorLayout);
	editorLayoutRef.current = editorLayout;

	// --- Extracted hooks ---
	const gitActions = useWorktreeGitActions({
		rootPath,
		stage,
		unstage,
		push,
		discard,
		createBranch,
		refreshGit,
		newBranchName,
		dispatchGit,
		dispatchUI,
		dispatchEditor,
	});

	useWorktreeMenuHandlers({
		editorLayout,
		activeTab: activeTab ?? null,
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
	});

	const {
		handleSendToTerminal,
		handleSendComment,
		handleCopyComment,
		handleCommentClick,
	} = useWorktreeComments({
		comments,
		addComment,
		removeComment,
		updateComment,
		markAsSent,
		activeTabPath,
		handleOpenFile,
		terminalRef,
		rootPath,
		dispatchEditor,
		commentsRef,
	});

	// --- Drag & drop ---
	const handleEditorDragOver = useCallback((e: React.DragEvent) => {
		if (e.dataTransfer.types.includes("Files")) {
			e.preventDefault();
			e.dataTransfer.dropEffect = "copy";
			dispatchUI({ type: "SET_EDITOR_DRAG_OVER", value: true });
		}
	}, []);

	const handleEditorDragLeave = useCallback((e: React.DragEvent) => {
		if (!e.currentTarget.contains(e.relatedTarget as Node)) {
			dispatchUI({ type: "SET_EDITOR_DRAG_OVER", value: false });
		}
	}, []);

	const handleEditorDrop = useCallback((e: React.DragEvent) => {
		e.preventDefault();
		dispatchUI({ type: "SET_EDITOR_DRAG_OVER", value: false });
	}, []);

	const centerTabRefInternal = centerTabRef;
	const { registerDropZone } = useNativeFileDrop({
		onDropToEditor: useCallback(
			(paths: string[]) => {
				dispatchUI({ type: "SET_EDITOR_DRAG_OVER", value: false });
				if (centerTabRefInternal?.current === "agent") return;
				for (const path of paths) {
					handleOpenFileRef.current(path);
				}
			},
			[centerTabRefInternal],
		),
	});

	const editorDropZoneRef = useCallback(
		(el: HTMLDivElement | null) => registerDropZone("editor", el),
		[registerDropZone],
	);

	// --- Monaco definition providers ---
	useEffect(() => {
		import("@monaco-editor/react")
			.then(({ loader }) => loader.init())
			.then((monaco) => {
				registerDefinitionProviders(monaco, {
					onOpenFileAtLine: (relativePath, line) => {
						const rp = rootPathRef.current;
						if (!rp) return;
						const absolutePath = `${rp}/${relativePath}`;
						handleOpenFileRef.current(absolutePath);
						dispatchEditor({
							type: "SET_PENDING_REVEAL",
							reveal: { path: absolutePath, line },
						});
					},
					getRootPath: () => rootPathRef.current,
				});
			})
			.catch((error) => {
				console.error("Failed to initialize Monaco:", error);
			});
	}, []);

	// --- Dirty sync ---
	const prevFilesRef = useRef(files);
	useEffect(() => {
		const prevFiles = prevFilesRef.current;
		for (const file of files) {
			const prev = prevFiles.find((f) => f.path === file.path);
			if (!prev || prev.isDirty !== file.isDirty) {
				editorLayout.updateTabDirty(file.path, file.isDirty);
			}
		}
		prevFilesRef.current = files;
	}, [files, editorLayout]);

	// --- Search handlers ---
	const handleSearchResultClick = useCallback(
		(relativePath: string, line: number) => {
			const absolutePath = `${rootPath}/${relativePath}`;
			handleOpenFile(absolutePath);
			dispatchEditor({
				type: "SET_PENDING_REVEAL",
				reveal: { path: absolutePath, line },
			});
		},
		[rootPath, handleOpenFile],
	);

	const handleSearchOccurrences = useCallback((text: string) => {
		dispatchEditor({ type: "TRIGGER_SEARCH", query: text });
	}, []);

	// --- Unsaved dialog handlers ---
	const handleUnsavedSave = useCallback(async () => {
		if (!closingTabPath) return;
		try {
			await saveFile(closingTabPath);
			closeFile(closingTabPath);
			editorLayout.removeTab(closingTabPath);
			dispatchUI({ type: "SET_CLOSING_TAB", path: null });
		} catch (e) {
			console.error("Failed to save file:", e);
			dispatchUI({ type: "SET_CLOSING_TAB", path: null });
		}
	}, [closingTabPath, saveFile, closeFile, editorLayout]);

	const handleUnsavedDiscard = useCallback(() => {
		if (!closingTabPath) return;
		closeFile(closingTabPath);
		editorLayout.removeTab(closingTabPath);
		dispatchUI({ type: "SET_CLOSING_TAB", path: null });
	}, [closingTabPath, closeFile, editorLayout]);

	const handleUnsavedCancel = useCallback(() => {
		dispatchUI({ type: "SET_CLOSING_TAB", path: null });
	}, []);

	// --- Rename / Delete ---
	const handleRename = useCallback(
		(oldPath: string, newPath: string) => {
			const file = getFileContent(oldPath);
			updateFilePath(oldPath, newPath);
			editorLayout.removeTab(oldPath);
			const newName = newPath.split(/[/\\]/).pop() ?? newPath;
			editorLayout.addTab(newPath, newName, file?.isDirty ?? false);
		},
		[updateFilePath, getFileContent, editorLayout],
	);

	const handleDelete = useCallback(
		(path: string) => {
			for (const file of files) {
				if (file.path === path || file.path.startsWith(`${path}/`)) {
					editorLayout.removeTab(file.path);
				}
			}
			closeFilesByPrefix(path);
		},
		[files, closeFilesByPrefix, editorLayout],
	);

	const closingTab = closingTabPath ? getFileContent(closingTabPath) : null;

	// --- EditorContext value ---
	const editorContextValue = useMemo<EditorContextValue>(
		() => ({
			getFileContent,
			updateContent,
			saveFile,
			diffBase,
			diffMode,
			setDiffBase,
			setDiffMode,
			comments,
			addComment,
			deleteComment: removeComment,
			updateComment,
			showSentComments,
			toggleShowSentComments,
			showInlineComments,
			toggleShowInlineComments,
			rootPath,
			onStageHunk: stageHunk,
			onGitChanged: refreshGit,
			gitRefreshKey,
			theme: settings.theme,
			fontSize: settings.fontSize,
			onSearchOccurrences: handleSearchOccurrences,
		}),
		[
			getFileContent,
			updateContent,
			saveFile,
			diffBase,
			diffMode,
			setDiffBase,
			setDiffMode,
			comments,
			addComment,
			removeComment,
			updateComment,
			showSentComments,
			toggleShowSentComments,
			showInlineComments,
			toggleShowInlineComments,
			rootPath,
			stageHunk,
			refreshGit,
			gitRefreshKey,
			settings.theme,
			settings.fontSize,
			handleSearchOccurrences,
		],
	);

	// --- Sidebar content ---
	const sidebarContent = useMemo(
		() => (
			<SidebarPanel
				rootPath={rootPath}
				onSelectFile={handleOpenFile}
				onFileChange={reloadFileIfClean}
				onRename={handleRename}
				onDelete={handleDelete}
				requestNewFolderKey={newFolderKey}
				activeTabPath={activeTabPath}
			/>
		),
		[
			rootPath,
			handleOpenFile,
			reloadFileIfClean,
			handleRename,
			handleDelete,
			newFolderKey,
			activeTabPath,
		],
	);

	return {
		ready,
		activeView,
		editorDragOver,
		comments,
		removeComment,
		updateComment,
		resolveComment,
		showSentComments,
		toggleShowSentComments,
		gitError,
		isSettingsOpen,
		closingTabPath,
		closingTab,
		savingConflictPath,
		showDiscardConfirm,
		showCreateBranch,
		newBranchName,
		branch,
		activeTab,
		agentState,
		editorLayout,
		terminalRef,
		dispatchEditor,
		dispatchUI,
		dispatchGit,
		gitActions,
		clearExternalChange,
		saveFile,
		handleEditorDragOver,
		handleEditorDragLeave,
		handleEditorDrop,
		editorDropZoneRef,
		handleSendToTerminal,
		handleSendComment,
		handleCopyComment,
		handleCommentClick,
		handleUnsavedSave,
		handleUnsavedDiscard,
		handleUnsavedCancel,
		editorContextValue,
		sidebarContent,
		refreshGit,
		gitRefreshKey,
		handleOpenFile,
		handleSearchResultClick,
		searchFocusKey,
		searchInitialQuery,
		pendingReveal,
		onSettingsSave,
		settings,
		rootPath,
	};
}
