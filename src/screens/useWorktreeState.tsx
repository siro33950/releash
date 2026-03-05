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
import { useBranchPr } from "@/hooks/useBranchPr";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useEditorLayout } from "@/hooks/useEditorLayout";
import { useFileContents } from "@/hooks/useFileContents";
import { type FileChangeEvent, useFileWatcher } from "@/hooks/useFileWatcher";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitDirWatcher } from "@/hooks/useGitDirWatcher";
import { useLsp } from "@/hooks/useLsp";
import { useLspMonaco } from "@/hooks/useLspMonaco";
import { useNativeFileDrop } from "@/hooks/useNativeFileDrop";
import { usePrDetail } from "@/hooks/usePrDetail";
import { usePrDiff } from "@/hooks/usePrDiff";
import { useThreadAI } from "@/hooks/useThreadAI";
import { useThreads } from "@/hooks/useThreads";
import { agentStateKey, aggregateAgentState } from "@/lib/agentStateUtils";
import {
	registerDefinitionProviders,
	setLspActive,
} from "@/lib/monaco-definition-provider";
import { normalizePath } from "@/lib/normalizePath";
import { useWorktreeThreads } from "@/screens/useWorktreeComments";
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
import { getThreadOrigin } from "@/types/thread";

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

	const [dismissedPrThreadIds, setDismissedPrThreadIds] = useState(
		() => new Set<string>(),
	);
	const [resolvedPrThreadIds, setResolvedPrThreadIds] = useState(
		() => new Set<string>(),
	);

	const mergedThreads = useMemo(() => {
		if (prDiff.reviewThreads.length === 0) return threads;
		const localIds = new Set(threads.map((t) => t.id));
		const prOnly = prDiff.reviewThreads
			.filter((t) => !localIds.has(t.id) && !dismissedPrThreadIds.has(t.id))
			.map((t) =>
				resolvedPrThreadIds.has(t.id) ? { ...t, resolved: true } : t,
			);
		return [...threads, ...prOnly];
	}, [
		threads,
		prDiff.reviewThreads,
		dismissedPrThreadIds,
		resolvedPrThreadIds,
	]);

	// Track which threads are using summarize mode
	const summarizeThreadIdsRef = useRef<Set<string>>(new Set());

	const [pendingPostToPr, setPendingPostToPr] = useState<{
		threadId: string;
		summary: string;
	} | null>(null);
	const [postToPrLoading, setPostToPrLoading] = useState(false);
	const [threadAIModalOpen, setThreadAIModalOpen] = useState(false);
	const [threadAIInitialThreadId, setThreadAIInitialThreadId] = useState<
		string | null
	>(null);

	const handleAICompleted = useCallback((threadId: string, _output: string) => {
		// AI posts its response via MCP add_thread_entry tool.
		// UI updates automatically via threads-changed event.
		if (summarizeThreadIdsRef.current.has(threadId)) {
			summarizeThreadIdsRef.current.delete(threadId);
			// For summarize, show the post-to-PR preview.
			const thread = threadsRef.current.find((t) => t.id === threadId);
			const lastAiEntry = thread?.entries
				.filter((e) => e.isAi)
				.sort((a, b) => b.createdAt - a.createdAt)[0];
			if (lastAiEntry) {
				setPendingPostToPr({ threadId, summary: lastAiEntry.content });
			}
		}
	}, []);
	const threadAI = useThreadAI(rootPath, settings, {
		onCompleted: handleAICompleted,
	});

	const aiRunningThreadIds = useMemo(() => {
		const ids = new Set<string>();
		for (const [threadId, task] of threadAI.taskMap) {
			if (task.status === "running") {
				ids.add(threadId);
			}
		}
		return ids;
	}, [threadAI.taskMap]);

	const aiTaskThreadIds = useMemo(
		() => new Set(threadAI.taskMap.keys()),
		[threadAI.taskMap],
	);

	const handleAskAI = useCallback(
		(threadId: string) => {
			console.log("[DEBUG] handleAskAI called with threadId:", threadId);
			threadAI.askAI(threadId, prNumber ?? undefined);
		},
		[threadAI, prNumber],
	);

	const handleOpenThreadAIModal = useCallback((threadId?: string) => {
		setThreadAIInitialThreadId(threadId ?? null);
		setThreadAIModalOpen(true);
	}, []);

	const handlePostToPr = useCallback(
		(threadId: string) => {
			summarizeThreadIdsRef.current.add(threadId);
			threadAI.summarizeForPr(threadId, prNumber ?? undefined);
		},
		[threadAI, prNumber],
	);

	const handlePostToPrConfirm = useCallback(
		async (editedSummary: string) => {
			if (!pendingPostToPr) return;
			setPostToPrLoading(true);
			try {
				const thread = mergedThreads.find(
					(t) => t.id === pendingPostToPr.threadId,
				);
				let postedComment: { id: number } | null = null;
				if (thread?.entries[0]?.prCommentId) {
					postedComment = await prDiff.replyToThread(
						pendingPostToPr.threadId,
						editedSummary,
					);
				} else {
					postedComment = await prDiff.postPrComment(editedSummary);
				}
				addEntry(
					pendingPostToPr.threadId,
					"Posted to PR",
					false,
					undefined,
					"posted-to-pr",
					postedComment?.id,
				);
			} finally {
				setPostToPrLoading(false);
				setPendingPostToPr(null);
			}
		},
		[pendingPostToPr, mergedThreads, prDiff, addEntry],
	);

	const handlePostToPrCancel = useCallback(() => {
		setPendingPostToPr(null);
	}, []);

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

	const threadsRef = useRef(threads);
	threadsRef.current = threads;

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
		handleSendThread,
		handleCopyThread,
		handleImplementThread,
		handleThreadClick,
	} = useWorktreeThreads({
		addEntry,
		resolveThread,
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

	// Wrap delete/resolve to also clean up AI tasks
	const handleDeleteThread = useCallback(
		(threadId: string) => {
			threadAI.removeTask(threadId);
			const thread = mergedThreads.find((t) => t.id === threadId);
			if (thread && getThreadOrigin(thread) === "pr") {
				setDismissedPrThreadIds((prev) => new Set(prev).add(threadId));
			} else {
				removeThread(threadId);
			}
		},
		[removeThread, threadAI, mergedThreads],
	);

	const handleResolveThread = useCallback(
		(threadId: string) => {
			threadAI.removeTask(threadId);
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
		[resolveThread, threadAI, mergedThreads],
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
				const thread = await createThread(
					filePath,
					lineNumber,
					content,
					endLine,
					undefined,
					undefined,
					undefined,
					fileContent,
				);
				handleAskAI(thread.id);
			},
			addEntry,
			deleteThread: handleDeleteThread,
			resolveThread: handleResolveThread,
			implementThread: handleImplementThread,
			onPostToPr: prNumber ? handlePostToPr : undefined,
			aiRunningThreadIds,
			aiTaskThreadIds,
			onOpenThreadAIModal: handleOpenThreadAIModal,
			onAskAI: handleAskAI,
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
			handleImplementThread,
			handleAskAI,
			prNumber,
			handlePostToPr,
			aiRunningThreadIds,
			aiTaskThreadIds,
			handleOpenThreadAIModal,
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
		threadAI,
		threadAIModalOpen,
		setThreadAIModalOpen,
		threadAIInitialThreadId,
		aiTaskThreadIds,
		handleOpenThreadAIModal,
		addEntry,
		pendingPostToPr,
		postToPrLoading,
		handlePostToPrConfirm,
		handlePostToPrCancel,
	};
}
