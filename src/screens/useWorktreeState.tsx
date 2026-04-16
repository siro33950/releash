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
import { useBranchPr } from "@/hooks/useBranchPr";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { pathFromTabId, useEditorLayout } from "@/hooks/useEditorLayout";
import { useFileContents } from "@/hooks/useFileContents";
import { type FileChangeEvent, useFileWatcher } from "@/hooks/useFileWatcher";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitDirWatcher } from "@/hooks/useGitDirWatcher";
import { useHandleOpenFile } from "@/hooks/useHandleOpenFile";
import { useLsp } from "@/hooks/useLsp";
import { useLspMonaco } from "@/hooks/useLspMonaco";
import { useNativeFileDrop } from "@/hooks/useNativeFileDrop";
import { usePrDetail } from "@/hooks/usePrDetail";
import { usePrDiff } from "@/hooks/usePrDiff";
import { useThreads } from "@/hooks/useThreads";
import { useWorkspaceStatus } from "@/hooks/useWorkspaceStatus";
import {
	registerDefinitionProviders,
	setLspActive,
} from "@/lib/monaco-definition-provider";
import { normalizePath } from "@/lib/normalizePath";
import { useWorktreeThreads } from "@/screens/useWorktreeComments";
import {
	createEditorState,
	editorReducer,
	gitReducer,
	initialUIState,
	uiReducer,
	useWorktreeGitActions,
} from "@/screens/useWorktreeGitActions";
import { useWorktreeMenuHandlers } from "@/screens/useWorktreeMenuHandlers";
import type { AppSettings, DiffBase, DiffMode } from "@/types/settings";
import { getThreadOrigin } from "@/types/thread";
import {
	type InternalWorktreeState,
	normalizeRightBottomActiveTab,
	type WorkspaceState,
} from "@/types/workspace-state";

export type { InternalWorktreeState } from "@/types/workspace-state";

interface UseWorktreeStateParams {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	isActive: boolean;
	centerTabRef?: React.RefObject<string>;
	onSwitchToEditor?: () => void;
	initialWorkspaceState?: WorkspaceState;
	internalStateMapRef?: React.MutableRefObject<
		Map<string, InternalWorktreeState>
	>;
}

export function useWorktreeState({
	rootPath,
	settings,
	onSettingsSave,
	isActive,
	centerTabRef,
	onSwitchToEditor,
	initialWorkspaceState,
	internalStateMapRef,
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
		createEditorState(
			initialWorkspaceState
				? { activeView: initialWorkspaceState.layout.activeView }
				: undefined,
		),
	);
	const {
		activeView,
		searchFocusKey,
		searchInitialQuery,
		pendingReveal,
		newFolderKey,
	} = editor;

	const [rightBottomCollapsed, setRightBottomCollapsed] = useState(
		initialWorkspaceState?.layout.rightBottomCollapsed ?? false,
	);

	const [rightBottomActiveTab, setRightBottomActiveTab] = useState<string>(
		normalizeRightBottomActiveTab(
			initialWorkspaceState?.layout.rightBottomActiveTab,
		),
	);

	const { branch } = useCurrentBranch(rootPath);
	const [ready, setReady] = useState(false);
	const {
		threads,
		createThread,
		addEntry,
		removeThread,
		updateEntry,
		resolveThread,
		showResolvedThreads,
		toggleShowResolvedThreads,
		recalculateAnchorsForFile,
	} = useThreads(rootPath);

	// --- PR integration ---
	const { prNumber } = useBranchPr(rootPath, branch);
	const { detail: prDetail } = usePrDetail(rootPath, prNumber);
	const prDiff = usePrDiff(
		rootPath,
		prNumber,
		prDetail?.base_ref_name ?? null,
		prDetail?.head_ref_name ?? null,
	);
	const { reviewThreads } = prDiff;

	const [dismissedPrThreadIds, setDismissedPrThreadIds] = useState(
		() => new Set<string>(),
	);
	const [resolvedPrThreadIds, setResolvedPrThreadIds] = useState(
		() => new Set<string>(),
	);

	const mergedThreads = useMemo(() => {
		if (reviewThreads.length === 0) return threads;
		const localIds = new Set(threads.map((t) => t.id));
		const prOnly = reviewThreads
			.filter((t) => !localIds.has(t.id) && !dismissedPrThreadIds.has(t.id))
			.map((t) =>
				resolvedPrThreadIds.has(t.id) ? { ...t, resolved: true } : t,
			);
		return [...threads, ...prOnly];
	}, [threads, reviewThreads, dismissedPrThreadIds, resolvedPrThreadIds]);

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

	const editorLayout = useEditorLayout(
		handleTabClose,
		initialWorkspaceState
			? {
					tabs: initialWorkspaceState.tabs.editors.map((e) => ({
						id: `editor:${e.path}`,
						path: e.path,
						name: e.name,
						isDirty: false,
						closable: true,
						draggable: true,
					})),
					activeTabId: initialWorkspaceState.tabs.activeEditorPath
						? `editor:${initialWorkspaceState.tabs.activeEditorPath}`
						: "",
				}
			: undefined,
	);
	const activeTabPath = editorLayout.getActiveTabPath();
	const activeTab = activeTabPath ? getFileContent(activeTabPath) : null;

	// --- LSP integration ---
	const activeTabLanguage = activeTab?.language ?? null;
	const lspLanguage = useMemo(() => {
		if (!activeTabLanguage) return null;
		// Normalize React variants and JavaScript to TypeScript for LSP
		if (
			activeTabLanguage === "typescriptreact" ||
			activeTabLanguage === "javascriptreact" ||
			activeTabLanguage === "javascript"
		) {
			return "typescript";
		}
		return activeTabLanguage;
	}, [activeTabLanguage]);

	const {
		transport: lspTransport,
		status: lspStatus,
		error: lspError,
		crashCount: lspCrashCount,
		retryManually: lspRetryManually,
	} = useLsp(rootPath, lspLanguage);

	const [monacoInstance, setMonacoInstance] = useState<
		typeof import("monaco-editor") | null
	>(null);
	const { connected: lspConnected } = useLspMonaco(
		monacoInstance,
		lspTransport,
	);

	// Track LSP active state for tree-sitter fallback
	useEffect(() => {
		if (lspLanguage && lspConnected) {
			setLspActive(lspLanguage, true);
			return () => setLspActive(lspLanguage, false);
		}
	}, [lspLanguage, lspConnected]);

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

	// --- Restore workspace state ---
	const initialWorkspaceStateRef = useRef(initialWorkspaceState);
	const openFileRef = useRef(openFile);
	openFileRef.current = openFile;
	useEffect(() => {
		const ws = initialWorkspaceStateRef.current;
		if (!ws) return;
		for (const tab of ws.tabs.editors) {
			openFileRef.current(tab.path);
		}
	}, []);

	// --- Lifecycle effects ---
	useEffect(() => {
		if (activeView === "git") refreshGit();
	}, [activeView, refreshGit]);

	useEffect(() => {
		if (branch != null) setReady(true);
	}, [branch]);

	// Rust 中央管理 (AgentStatusCenter) から worktree 集約状態を購読する。
	// 派生計算はすべて Rust 側 (`AgentStatusCenter::aggregate`) で完結する。
	const workspaceStatus = useWorkspaceStatus(rootPath);
	const agentState = workspaceStatus?.aggregated_state;

	// --- Refs ---
	const rootPathRef = useRef(rootPath);
	rootPathRef.current = rootPath;

	const threadsRef = useRef(threads);
	threadsRef.current = threads;

	const settingsRef = useRef(settings);
	settingsRef.current = settings;

	const onSettingsSaveRef = useRef(onSettingsSave);
	onSettingsSaveRef.current = onSettingsSave;

	// --- File open sync ---
	const handleOpenFile = useHandleOpenFile({
		openFile,
		getFileContent,
		addTab: editorLayout.addTab,
		onSwitchToEditor,
	});
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
		handleSendThread,
		handleCopyThread,
		handleThreadClick,
	} = useWorktreeThreads({
		activeTabPath,
		handleOpenFile,
		terminalRef,
		rootPath,
		dispatchEditor,
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
				setMonacoInstance(monaco);
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

	const handleDeleteThread = useCallback(
		(threadId: string) => {
			const thread = mergedThreads.find((t) => t.id === threadId);
			if (thread && getThreadOrigin(thread) === "pr") {
				setDismissedPrThreadIds((prev) => new Set(prev).add(threadId));
			} else {
				removeThread(threadId);
			}
		},
		[removeThread, mergedThreads],
	);

	const handleResolveThread = useCallback(
		(threadId: string) => {
			const thread = mergedThreads.find((t) => t.id === threadId);
			if (thread && getThreadOrigin(thread) === "pr") {
				setResolvedPrThreadIds((prev) => {
					const next = new Set(prev);
					if (next.has(threadId)) {
						next.delete(threadId);
					} else {
						next.add(threadId);
					}
					return next;
				});
			} else {
				resolveThread(threadId);
			}
		},
		[resolveThread, mergedThreads],
	);

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
			threads: mergedThreads,
			createThread: async (
				filePath,
				lineNumber,
				content,
				endLine?,
				fileContent?,
			) => {
				await createThread(
					filePath,
					lineNumber,
					content,
					endLine,
					undefined,
					undefined,
					fileContent,
				);
			},
			addEntry,
			deleteThread: handleDeleteThread,
			resolveThread: handleResolveThread,
			updateEntry,
			sendThread: handleSendThread,
			copyThread: handleCopyThread,
			recalculateAnchorsForFile,
			showResolvedThreads,
			toggleShowResolvedThreads,
			rootPath,
			onStageHunk: stageHunk,
			onGitChanged: refreshGit,
			gitRefreshKey,
			theme: settings.theme,
			fontSize: settings.fontSize,
			onSearchOccurrences: handleSearchOccurrences,
			lspStatus,
			lspError,
			lspCrashCount,
			lspRetryManually,
		}),
		[
			getFileContent,
			updateContent,
			saveFile,
			diffBase,
			diffMode,
			setDiffBase,
			setDiffMode,
			mergedThreads,
			createThread,
			addEntry,
			handleDeleteThread,
			handleResolveThread,
			updateEntry,
			handleSendThread,
			handleCopyThread,
			recalculateAnchorsForFile,
			showResolvedThreads,
			toggleShowResolvedThreads,
			rootPath,
			stageHunk,
			refreshGit,
			gitRefreshKey,
			settings.theme,
			settings.fontSize,
			handleSearchOccurrences,
			lspStatus,
			lspError,
			lspCrashCount,
			lspRetryManually,
		],
	);

	// --- Sync internal state for workspace state persistence ---
	useEffect(() => {
		if (!internalStateMapRef) return;
		internalStateMapRef.current.set(rootPath, {
			tabs: editorLayout.tabs
				.filter((t) => t.path != null)
				.map((t) => ({
					path: t.path as string,
					name: t.name,
				})),
			activeEditorPath: pathFromTabId(editorLayout.activeTabId),
			activeView,
			rightBottomCollapsed,
			rightBottomActiveTab,
		});
	}, [
		internalStateMapRef,
		rootPath,
		editorLayout.tabs,
		editorLayout.activeTabId,
		activeView,
		rightBottomCollapsed,
		rightBottomActiveTab,
	]);

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
		threads: mergedThreads,
		removeThread: handleDeleteThread,
		updateEntry,
		resolveThread: handleResolveThread,
		showResolvedThreads,
		toggleShowResolvedThreads,
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
		handleSendThread,
		handleCopyThread,
		handleThreadClick,
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
		lspStatus,
		lspCrashCount,
		lspRetryManually,
		addEntry,
		rightBottomCollapsed,
		setRightBottomCollapsed,
		rightBottomActiveTab,
		setRightBottomActiveTab,
	};
}
