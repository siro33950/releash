import { FileIcon } from "@react-symbols/icons/utils";
import { listen } from "@tauri-apps/api/event";
import type { ITabRenderValues, TabNode } from "flexlayout-react";
import { PanelBottom, PanelLeft, PanelRight } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useReducer,
	useRef,
	useState,
} from "react";
import type { PanelImperativeHandle, PanelSize } from "react-resizable-panels";
import type { TogglePanel } from "@/components/layout/ViewToolbar";
import { EditorTabContent } from "@/components/panels/EditorTabContent";
import { EmptyState } from "@/components/panels/EmptyState";
import { PullRequestPanel } from "@/components/panels/PullRequestPanel";
import { SearchPanel } from "@/components/panels/SearchPanel";
import { SidebarPanel } from "@/components/panels/SidebarPanel";
import { SourceControlPanel } from "@/components/panels/SourceControlPanel";
import type { TerminalTabPanelHandle } from "@/components/panels/TerminalTabPanel";
import type { EditorContextValue } from "@/contexts/EditorContext";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useEditorLayout } from "@/hooks/useEditorLayout";
import { useFileContents } from "@/hooks/useFileContents";
import { type FileChangeEvent, useFileWatcher } from "@/hooks/useFileWatcher";
import { useGitActions } from "@/hooks/useGitActions";
import { useLineComments } from "@/hooks/useLineComments";
import { useNativeFileDrop } from "@/hooks/useNativeFileDrop";
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
import type { AgentState, AgentStateSync } from "@/types/protocol";
import type { AppSettings, DiffBase, DiffMode } from "@/types/settings";

interface UseWorktreeStateParams {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	isActive: boolean;
}

export function useWorktreeState({
	rootPath,
	settings,
	onSettingsSave,
	isActive,
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
	const [agentState, setAgentState] = useState<AgentState | undefined>();
	const {
		comments,
		addComment,
		removeComment,
		updateComment,
		markAsSent,
		showSentComments,
		toggleShowSentComments,
	} = useLineComments();
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
	const [, forceRender] = useReducer((x: number) => x + 1, 0);
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
				setAgentState(event.payload.state);
			}
		});
		return () => {
			unlisten.then((f) => f());
		};
	}, [rootPath]);

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

	const { registerDropZone } = useNativeFileDrop({
		onDropToEditor: useCallback((paths: string[]) => {
			dispatchUI({ type: "SET_EDITOR_DRAG_OVER", value: false });
			for (const path of paths) {
				handleOpenFileRef.current(path);
			}
		}, []),
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
			rootPath,
			stageHunk,
			refreshGit,
			gitRefreshKey,
			settings.theme,
			settings.fontSize,
			handleSearchOccurrences,
		],
	);

	// --- Factory ---
	const factory = useCallback(
		(node: TabNode): ReactNode => {
			const component = node.getComponent();
			if (component === "editor") {
				const config = node.getConfig();
				const filePath = config?.filePath;
				if (!filePath)
					return (
						<EmptyState
							title="No file selected"
							description="Select a file from the explorer to view its contents"
						/>
					);
				return (
					<EditorTabContent
						key={filePath}
						filePath={filePath}
						externalRevealLine={pendingReveal}
						onExternalRevealConsumed={() =>
							dispatchEditor({ type: "SET_PENDING_REVEAL", reveal: null })
						}
					/>
				);
			}
			return null;
		},
		[pendingReveal],
	);

	// --- Sidebar content ---
	const sidebarContent = useMemo(() => {
		if (activeView === "git") {
			return (
				<SourceControlPanel
					rootPath={rootPath}
					onSelectFile={handleOpenFile}
					onGitChanged={refreshGit}
					gitRefreshKey={gitRefreshKey}
				/>
			);
		}
		if (activeView === "search") {
			return (
				<SearchPanel
					rootPath={rootPath}
					onSelectFileAtLine={handleSearchResultClick}
					focusKey={searchFocusKey}
					initialQuery={searchInitialQuery}
				/>
			);
		}
		if (activeView === "pr") {
			return <PullRequestPanel rootPath={rootPath} branch={branch} />;
		}
		return (
			<SidebarPanel
				rootPath={rootPath}
				onSelectFile={handleOpenFile}
				onFileChange={reloadFileIfClean}
				onRename={handleRename}
				onDelete={handleDelete}
				requestNewFolderKey={newFolderKey}
				activeTabPath={activeTabPath}
			/>
		);
	}, [
		activeView,
		rootPath,
		handleOpenFile,
		refreshGit,
		gitRefreshKey,
		handleSearchResultClick,
		searchFocusKey,
		searchInitialQuery,
		reloadFileIfClean,
		handleRename,
		handleDelete,
		newFolderKey,
		branch,
		activeTabPath,
	]);

	// --- Panel toggle ---
	const sidebarPanelRef = useRef<PanelImperativeHandle>(null);
	const reviewPanelRef = useRef<PanelImperativeHandle>(null);
	const terminalPanelRef = useRef<PanelImperativeHandle>(null);

	const [sidebarVisible, setSidebarVisible] = useState(true);
	const [reviewVisible, setReviewVisible] = useState(true);
	const [terminalVisible, setTerminalVisible] = useState(true);

	const handleSidebarResize = useCallback((size: PanelSize) => {
		const visible = size.asPercentage > 0;
		setSidebarVisible((prev) => (prev === visible ? prev : visible));
	}, []);
	const handleReviewResize = useCallback((size: PanelSize) => {
		const visible = size.asPercentage > 0;
		setReviewVisible((prev) => (prev === visible ? prev : visible));
	}, []);
	const handleTerminalResize = useCallback((size: PanelSize) => {
		const visible = size.asPercentage > 0;
		setTerminalVisible((prev) => (prev === visible ? prev : visible));
	}, []);

	const toggleSidebar = useCallback(() => {
		const panel = sidebarPanelRef.current;
		if (!panel) return;
		panel.isCollapsed() ? panel.expand() : panel.collapse();
	}, []);
	const toggleReview = useCallback(() => {
		const panel = reviewPanelRef.current;
		if (!panel) return;
		panel.isCollapsed() ? panel.expand() : panel.collapse();
	}, []);
	const toggleTerminal = useCallback(() => {
		const panel = terminalPanelRef.current;
		if (!panel) return;
		panel.isCollapsed() ? panel.expand() : panel.collapse();
	}, []);

	const togglePanels = useMemo<TogglePanel[]>(
		() => [
			{
				id: "sidebar",
				icon: PanelLeft,
				label: "Sidebar",
				visible: sidebarVisible,
				onToggle: toggleSidebar,
			},
			{
				id: "review",
				icon: PanelBottom,
				label: "Review",
				visible: reviewVisible,
				onToggle: toggleReview,
			},
			{
				id: "terminal",
				icon: PanelRight,
				label: "Terminal",
				visible: terminalVisible,
				onToggle: toggleTerminal,
			},
		],
		[
			sidebarVisible,
			reviewVisible,
			terminalVisible,
			toggleSidebar,
			toggleReview,
			toggleTerminal,
		],
	);

	// --- Tab rendering ---
	const onRenderTab = useCallback(
		(node: TabNode, renderValues: ITabRenderValues) => {
			if (node.getComponent() === "editor") {
				const config = node.getConfig();
				renderValues.leading = (
					<FileIcon fileName={node.getName()} className="h-4 w-4" />
				);
				if (config?.isDirty) {
					renderValues.buttons.push(
						<span
							key="dirty"
							className="w-2 h-2 rounded-full bg-foreground shrink-0"
						/>,
					);
				}
			}
		},
		[],
	);

	return {
		ready,
		activeView,
		editorDragOver,
		comments,
		removeComment,
		updateComment,
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
		forceRender,
		terminalRef,
		sidebarPanelRef,
		reviewPanelRef,
		terminalPanelRef,
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
		handleSidebarResize,
		handleReviewResize,
		handleTerminalResize,
		togglePanels,
		editorContextValue,
		sidebarContent,
		factory,
		onRenderTab,
		onSettingsSave,
		settings,
		rootPath,
	};
}
