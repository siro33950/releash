import { loader } from "@monaco-editor/react";
import { FileIcon } from "@react-symbols/icons/utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { type ITabRenderValues, Layout, type TabNode } from "flexlayout-react";
import { Loader2, PanelBottom, PanelLeft, PanelRight } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useReducer,
	useRef,
	useState,
} from "react";
import {
	Group,
	Panel,
	type PanelImperativeHandle,
	type PanelSize,
	Separator,
} from "react-resizable-panels";
import { ActivityBar } from "@/components/layout/ActivityBar";
import { StatusBar } from "@/components/layout/StatusBar";
import { type TogglePanel, ViewToolbar } from "@/components/layout/ViewToolbar";
import { EditorTabContent } from "@/components/panels/EditorTabContent";
import { EmptyState } from "@/components/panels/EmptyState";
import { PullRequestPanel } from "@/components/panels/PullRequestPanel";
import { ReviewPanel } from "@/components/panels/ReviewPanel";
import { SearchPanel } from "@/components/panels/SearchPanel";
import { SettingsPanel } from "@/components/panels/SettingsPanel";
import { SidebarPanel } from "@/components/panels/SidebarPanel";
import { SourceControlPanel } from "@/components/panels/SourceControlPanel";
import {
	TerminalPanel,
	type TerminalPanelHandle,
} from "@/components/panels/TerminalPanel";
import { UnsavedChangesDialog } from "@/components/panels/UnsavedChangesDialog";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Input } from "@/components/ui/input";
import {
	EditorContext,
	type EditorContextValue,
} from "@/contexts/EditorContext";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useEditorLayout } from "@/hooks/useEditorLayout";
import { useFileContents } from "@/hooks/useFileContents";
import { type FileChangeEvent, useFileWatcher } from "@/hooks/useFileWatcher";
import { useGitActions } from "@/hooks/useGitActions";
import { useLineComments } from "@/hooks/useLineComments";
import { type MenuHandlers, useMenuEvents } from "@/hooks/useMenuEvents";
import { useNativeFileDrop } from "@/hooks/useNativeFileDrop";
import { formatCommentForClipboard } from "@/lib/formatCommentForClipboard";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import { registerDefinitionProviders } from "@/lib/monaco-definition-provider";
import { normalizePath } from "@/lib/normalizePath";
import { trackEvent } from "@/lib/telemetry";
import type { LineComment } from "@/types/comment";
import type { AgentState, AgentStateSync } from "@/types/protocol";
import {
	type AppSettings,
	buildTerminalCommand,
	type DiffBase,
	type DiffMode,
} from "@/types/settings";

interface WorktreeViewProps {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	isActive: boolean;
}

export function WorktreeView({
	rootPath,
	settings,
	onSettingsSave,
	isActive,
}: WorktreeViewProps) {
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

	const [activeView, setActiveView] = useState<string>("git");
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
	const terminalRef = useRef<TerminalPanelHandle>(null);
	const [gitRefreshKey, setGitRefreshKey] = useState(0);
	const refreshGit = useCallback(() => setGitRefreshKey((k) => k + 1), []);

	const [diffBase, setDiffBase] = useState<DiffBase>(settings.defaultDiffBase);
	const [diffMode, setDiffMode] = useState<DiffMode>(settings.defaultDiffMode);
	const [closingTabPath, setClosingTabPath] = useState<string | null>(null);
	const [savingConflictPath, setSavingConflictPath] = useState<string | null>(
		null,
	);
	const [pendingReveal, setPendingReveal] = useState<{
		path: string;
		line: number;
	} | null>(null);
	const [searchFocusKey, setSearchFocusKey] = useState(0);

	const handleTabClose = useCallback(
		(path: string): boolean => {
			const file = getFileContent(path);
			if (file?.isDirty) {
				setClosingTabPath(path);
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

	useEffect(() => {
		if (activeView === "git") {
			refreshGit();
		}
	}, [activeView, refreshGit]);

	useEffect(() => {
		if (branch != null) {
			setReady(true);
		}
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

	const rootPathRef = useRef(rootPath);
	rootPathRef.current = rootPath;

	const commentsRef = useRef(comments);
	commentsRef.current = comments;

	const settingsRef = useRef(settings);
	settingsRef.current = settings;

	const onSettingsSaveRef = useRef(onSettingsSave);
	onSettingsSaveRef.current = onSettingsSave;

	const [editorDragOver, setEditorDragOver] = useState(false);

	const handleEditorDragOver = useCallback((e: React.DragEvent) => {
		if (e.dataTransfer.types.includes("Files")) {
			e.preventDefault();
			e.dataTransfer.dropEffect = "copy";
			setEditorDragOver(true);
		}
	}, []);

	const handleEditorDragLeave = useCallback((e: React.DragEvent) => {
		if (!e.currentTarget.contains(e.relatedTarget as Node)) {
			setEditorDragOver(false);
		}
	}, []);

	const handleEditorDrop = useCallback((e: React.DragEvent) => {
		e.preventDefault();
		setEditorDragOver(false);
	}, []);

	const [newFolderKey, setNewFolderKey] = useState(0);
	const [gitError, setGitError] = useState<string | null>(null);
	const [showDiscardConfirm, setShowDiscardConfirm] = useState(false);
	const [showCreateBranch, setShowCreateBranch] = useState(false);
	const [newBranchName, setNewBranchName] = useState("");

	const broadcastComments = useCallback((commentsList: LineComment[]) => {
		invoke("broadcast_comments", {
			comments: {
				comments: commentsList.map((c) => ({
					id: c.id,
					file_path: c.filePath,
					line_number: c.lineNumber,
					...(c.endLine != null && { end_line: c.endLine }),
					content: c.content,
					status: c.status,
					created_at: c.createdAt,
				})),
			},
		}).catch(() => {});
	}, []);

	useEffect(() => {
		const unlistenComment = listen<{
			file_path: string;
			line_number: number;
			end_line?: number;
			content: string;
		}>("remote-comment-added", (event) => {
			const { file_path, line_number, end_line, content } = event.payload;
			addComment(file_path, line_number, content, end_line ?? undefined);
		});

		const unlistenDelete = listen<{ id: string }>(
			"remote-comment-deleted",
			(event) => {
				removeComment(event.payload.id);
			},
		);

		const unlistenUpdate = listen<{ id: string; content: string }>(
			"remote-comment-updated",
			(event) => {
				updateComment(event.payload.id, event.payload.content);
			},
		);

		const unlistenConnected = listen("remote-connected", () => {
			broadcastComments(commentsRef.current);
		});

		return () => {
			unlistenComment.then((f) => f());
			unlistenDelete.then((f) => f());
			unlistenUpdate.then((f) => f());
			unlistenConnected.then((f) => f());
		};
	}, [addComment, removeComment, updateComment, broadcastComments]);

	useEffect(() => {
		broadcastComments(comments);
	}, [comments, broadcastComments]);

	useEffect(() => {
		loader
			.init()
			.then((monaco) => {
				registerDefinitionProviders(monaco, {
					onOpenFileAtLine: (relativePath, line) => {
						const rp = rootPathRef.current;
						if (!rp) return;
						const absolutePath = `${rp}/${relativePath}`;
						handleOpenFileRef.current(absolutePath);
						setPendingReveal({ path: absolutePath, line });
					},
					getRootPath: () => rootPathRef.current,
				});
			})
			.catch((error) => {
				console.error("Failed to initialize Monaco:", error);
			});
	}, []);

	// Sync file open: opens in both useFileContents and flexlayout
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

	const { registerDropZone } = useNativeFileDrop({
		onDropToEditor: useCallback((paths: string[]) => {
			setEditorDragOver(false);
			for (const path of paths) {
				handleOpenFileRef.current(path);
			}
		}, []),
	});

	const editorDropZoneRef = useCallback(
		(el: HTMLDivElement | null) => registerDropZone("editor", el),
		[registerDropZone],
	);

	// Sync dirty state to flexlayout tab
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

	const handleSave = useCallback(() => {
		if (!activeTab?.isDirty) return;
		if (activeTab.hasExternalChange) {
			setSavingConflictPath(activeTab.path);
		} else {
			saveFile(activeTab.path);
		}
	}, [activeTab, saveFile]);

	const handleSearch = useCallback(() => {
		setActiveView("search");
		setSearchFocusKey((k) => k + 1);
	}, []);

	const handleCloseActiveTab = useCallback(() => {
		if (activeTab) {
			if (activeTab.isDirty) {
				setClosingTabPath(activeTab.path);
			} else {
				closeFile(activeTab.path);
				editorLayout.removeTab(activeTab.path);
			}
		}
	}, [activeTab, closeFile, editorLayout]);

	const handleGitStageAll = useCallback(async () => {
		try {
			const status = await invoke<{ changed: Array<{ path: string }> }>(
				"get_git_status",
				{ repoPath: rootPath },
			);
			if (status.changed.length > 0) {
				await stage(
					rootPath,
					status.changed.map((f) => f.path),
				);
				refreshGit();
			}
		} catch (e) {
			setGitError(String(e));
		}
	}, [rootPath, stage, refreshGit]);

	const handleGitUnstageAll = useCallback(async () => {
		try {
			const status = await invoke<{ staged: Array<{ path: string }> }>(
				"get_git_status",
				{ repoPath: rootPath },
			);
			if (status.staged.length > 0) {
				await unstage(
					rootPath,
					status.staged.map((f) => f.path),
				);
				refreshGit();
			}
		} catch (e) {
			setGitError(String(e));
		}
	}, [rootPath, unstage, refreshGit]);

	const handleGitCommit = useCallback(() => {
		setActiveView("git");
	}, []);

	const handleGitPush = useCallback(async () => {
		try {
			await push(rootPath);
		} catch (e) {
			setGitError(String(e));
		}
	}, [rootPath, push]);

	const handleGitDiscardAll = useCallback(() => {
		setShowDiscardConfirm(true);
	}, []);

	const executeDiscardAll = useCallback(async () => {
		setShowDiscardConfirm(false);
		try {
			const status = await invoke<{ changed: Array<{ path: string }> }>(
				"get_git_status",
				{ repoPath: rootPath },
			);
			if (status.changed.length > 0) {
				await discard(
					rootPath,
					status.changed.map((f) => f.path),
				);
				refreshGit();
			}
		} catch (e) {
			setGitError(String(e));
		}
	}, [rootPath, discard, refreshGit]);

	const handleGitCreateBranch = useCallback(() => {
		setNewBranchName("");
		setShowCreateBranch(true);
	}, []);

	const executeCreateBranch = useCallback(async () => {
		const name = newBranchName.trim();
		if (!name) return;
		setShowCreateBranch(false);
		try {
			await createBranch(rootPath, name);
		} catch (e) {
			setGitError(String(e));
		}
	}, [rootPath, createBranch, newBranchName]);

	const handleCreateUntitledTab = useCallback(() => {
		const path = createUntitledFile();
		const name = path.split(":").pop() ?? path;
		editorLayout.addTab(path, name, true);
	}, [createUntitledFile, editorLayout]);

	const menuHandlers: MenuHandlers = useMemo(
		() => ({
			"new-file": handleCreateUntitledTab,
			"new-folder": () => {
				setActiveView("explorer");
				setNewFolderKey((k) => k + 1);
			},
			save: handleSave,
			"save-all": saveAllDirtyFiles,
			"close-tab": handleCloseActiveTab,
			"close-all-tabs": () => {
				closeAllFiles();
				// Remove all editor tabs from flexlayout
				for (const file of files) {
					editorLayout.removeTab(file.path);
				}
			},
			"find-in-files": handleSearch,
			"view-explorer": () => setActiveView("explorer"),
			"view-search": () => {
				setActiveView("search");
				setSearchFocusKey((k) => k + 1);
			},
			"view-source-control": () => setActiveView("git"),
			settings: () => setActiveView("settings"),
			"diff-gutter": () => setDiffMode("gutter"),
			"diff-inline": () => setDiffMode("inline"),
			"diff-split": () => setDiffMode("split"),
			"increase-font-size": () => {
				const s = settingsRef.current;
				onSettingsSaveRef.current({ ...s, fontSize: s.fontSize + 1 });
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
			"git-stage-all": handleGitStageAll,
			"git-unstage-all": handleGitUnstageAll,
			"git-commit": handleGitCommit,
			"git-push": handleGitPush,
			"git-discard-all": handleGitDiscardAll,
			"git-create-branch": handleGitCreateBranch,
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
			handleGitStageAll,
			handleGitUnstageAll,
			handleGitCommit,
			handleGitPush,
			handleGitDiscardAll,
			handleGitCreateBranch,
		],
	);

	useMenuEvents(menuHandlers, isActive);

	const handleSearchResultClick = useCallback(
		(relativePath: string, line: number) => {
			const absolutePath = `${rootPath}/${relativePath}`;
			handleOpenFile(absolutePath);
			setPendingReveal({ path: absolutePath, line });
		},
		[rootPath, handleOpenFile],
	);

	const handleSearchOccurrences = useCallback((_text: string) => {
		setActiveView("search");
		setSearchFocusKey((k) => k + 1);
	}, []);

	const handleUnsavedSave = useCallback(async () => {
		if (!closingTabPath) return;
		try {
			await saveFile(closingTabPath);
			closeFile(closingTabPath);
			editorLayout.removeTab(closingTabPath);
			setClosingTabPath(null);
		} catch (e) {
			console.error("Failed to save file:", e);
			setClosingTabPath(null);
		}
	}, [closingTabPath, saveFile, closeFile, editorLayout]);

	const handleUnsavedDiscard = useCallback(() => {
		if (!closingTabPath) return;
		closeFile(closingTabPath);
		editorLayout.removeTab(closingTabPath);
		setClosingTabPath(null);
	}, [closingTabPath, closeFile, editorLayout]);

	const handleUnsavedCancel = useCallback(() => {
		setClosingTabPath(null);
	}, []);

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

	const handleSendToTerminal = useCallback(
		(unsent: LineComment[]) => {
			const text = formatCommentsForTerminal(unsent, rootPath);
			if (text && terminalRef.current) {
				terminalRef.current.writeToTerminal(`${text}\n`);
				markAsSent(unsent.map((c) => c.id));
				trackEvent("comment_sent", { count: unsent.length });
			}
		},
		[markAsSent, rootPath],
	);

	const handleSendComment = useCallback(
		(comment: LineComment) => {
			const text = formatCommentsForTerminal([comment], rootPath);
			if (text && terminalRef.current) {
				terminalRef.current.writeToTerminal(`${text}\n`);
				markAsSent([comment.id]);
				trackEvent("comment_sent", { count: 1 });
			}
		},
		[markAsSent, rootPath],
	);

	const handleCopyComment = useCallback((comment: LineComment) => {
		const text = formatCommentForClipboard(comment);
		navigator.clipboard.writeText(text).catch(() => {});
		trackEvent("comment_copied");
	}, []);

	const handleCommentClick = useCallback(
		(commentFilePath: string, lineNumber: number) => {
			if (activeTabPath === commentFilePath) {
				setPendingReveal({ path: commentFilePath, line: lineNumber });
			} else {
				handleOpenFile(commentFilePath);
				setPendingReveal({ path: commentFilePath, line: lineNumber });
			}
		},
		[activeTabPath, handleOpenFile],
	);

	const closingTab = closingTabPath ? getFileContent(closingTabPath) : null;

	// EditorContext value
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

	// flexlayout factory: renders the content of each editor tab
	const factory = useCallback(
		(node: TabNode): ReactNode => {
			const component = node.getComponent();
			if (component === "editor") {
				const config = node.getConfig();
				const filePath = config?.filePath;
				if (!filePath) return <EmptyState />;
				return (
					<EditorTabContent
						filePath={filePath}
						externalRevealLine={pendingReveal}
						onExternalRevealConsumed={() => setPendingReveal(null)}
					/>
				);
			}
			return null;
		},
		[pendingReveal],
	);

	// Sidebar content based on activeView
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
				/>
			);
		}
		if (activeView === "pr") {
			return <PullRequestPanel rootPath={rootPath} branch={branch} />;
		}
		if (activeView === "settings") {
			return <SettingsPanel settings={settings} onSave={onSettingsSave} />;
		}
		return (
			<SidebarPanel
				rootPath={rootPath}
				onSelectFile={handleOpenFile}
				onFileChange={reloadFileIfClean}
				onRename={handleRename}
				onDelete={handleDelete}
				requestNewFolderKey={newFolderKey}
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
		settings,
		onSettingsSave,
		reloadFileIfClean,
		handleRename,
		handleDelete,
		newFolderKey,
		branch,
	]);

	// Panel refs and visibility state
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
		if (panel.isCollapsed()) {
			panel.expand();
		} else {
			panel.collapse();
		}
	}, []);

	const toggleReview = useCallback(() => {
		const panel = reviewPanelRef.current;
		if (!panel) return;
		if (panel.isCollapsed()) {
			panel.expand();
		} else {
			panel.collapse();
		}
	}, []);

	const toggleTerminal = useCallback(() => {
		const panel = terminalPanelRef.current;
		if (!panel) return;
		if (panel.isCollapsed()) {
			panel.expand();
		} else {
			panel.collapse();
		}
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

	// Custom tab rendering for editor tabs
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

	return (
		<div className="flex flex-col h-full w-full overflow-hidden bg-background text-foreground">
			<ViewToolbar panels={togglePanels} />
			<div className="flex flex-1 overflow-hidden">
				<ActivityBar activeItem={activeView} onItemClick={setActiveView} />
				{!ready ? (
					<div className="flex-1 flex items-center justify-center">
						<Loader2 className="size-6 text-muted-foreground animate-spin" />
					</div>
				) : (
					<EditorContext.Provider value={editorContextValue}>
						<Group orientation="horizontal" className="flex-1">
							<Panel
								panelRef={sidebarPanelRef}
								id="sidebar"
								defaultSize="20%"
								minSize="10%"
								collapsible
								collapsedSize="0%"
								onResize={handleSidebarResize}
							>
								<div className="h-full overflow-hidden border-r border-border">
									{sidebarContent}
								</div>
							</Panel>
							<Separator />
							<Panel id="center" minSize="20%">
								<Group orientation="vertical">
									<Panel id="editor" minSize="20%">
										<div
											ref={editorDropZoneRef}
											role="application"
											className="h-full relative overflow-hidden"
											onDragOver={handleEditorDragOver}
											onDragLeave={handleEditorDragLeave}
											onDrop={handleEditorDrop}
										>
											<Layout
												model={editorLayout.model}
												factory={factory}
												onAction={editorLayout.onAction}
												onRenderTab={onRenderTab}
												onModelChange={forceRender}
											/>
											{editorDragOver && (
												<div className="absolute inset-0 flex items-center justify-center bg-primary/10 border-2 border-dashed border-primary rounded pointer-events-none">
													<span className="text-sm font-medium text-primary bg-background/80 px-3 py-1.5 rounded">
														ドロップしてファイルを開く
													</span>
												</div>
											)}
										</div>
									</Panel>
									<Separator />
									<Panel
										panelRef={reviewPanelRef}
										id="review"
										defaultSize="30%"
										minSize="10%"
										collapsible
										collapsedSize="0%"
										onResize={handleReviewResize}
									>
										<div className="h-full overflow-hidden border-t border-border">
											<ReviewPanel
												comments={comments}
												onCommentClick={handleCommentClick}
												onDeleteComment={removeComment}
												onUpdateComment={updateComment}
												onSendToTerminal={handleSendToTerminal}
												onSendComment={handleSendComment}
												onCopyComment={handleCopyComment}
												showSentComments={showSentComments}
												onToggleShowSent={toggleShowSentComments}
												cwd={rootPath}
												theme={settings.theme}
											/>
										</div>
									</Panel>
								</Group>
							</Panel>
							<Separator />
							<Panel
								panelRef={terminalPanelRef}
								id="terminal"
								defaultSize="30%"
								minSize="10%"
								collapsible
								collapsedSize="0%"
								onResize={handleTerminalResize}
							>
								<div className="h-full overflow-hidden border-l border-border">
									<TerminalPanel
										ref={terminalRef}
										key={rootPath}
										cwd={rootPath}
										theme={settings.theme}
										terminalStartupCommand={buildTerminalCommand(settings)}
										agentType={settings.agent}
									/>
								</div>
							</Panel>
						</Group>
					</EditorContext.Provider>
				)}
			</div>
			<StatusBar
				className="shrink-0"
				branch={branch ?? undefined}
				language={activeTab?.language}
				encoding={activeTab ? "UTF-8" : undefined}
				eol={activeTab?.eol}
				agentState={agentState}
			/>
			<UnsavedChangesDialog
				open={!!closingTabPath}
				fileName={closingTab?.name ?? ""}
				onSave={handleUnsavedSave}
				onDiscard={handleUnsavedDiscard}
				onCancel={handleUnsavedCancel}
			/>
			<AlertDialog
				open={!!savingConflictPath}
				onOpenChange={(o) => {
					if (!o) setSavingConflictPath(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>External Change Conflict</AlertDialogTitle>
						<AlertDialogDescription>
							This file has been modified externally. Do you want to overwrite
							it?
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel onClick={() => setSavingConflictPath(null)}>
							Cancel
						</AlertDialogCancel>
						<AlertDialogAction
							onClick={() => {
								if (savingConflictPath) {
									clearExternalChange(savingConflictPath);
									saveFile(savingConflictPath);
								}
								setSavingConflictPath(null);
							}}
						>
							Overwrite
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
			<AlertDialog
				open={!!gitError}
				onOpenChange={(o) => {
					if (!o) setGitError(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Git Error</AlertDialogTitle>
						<AlertDialogDescription>{gitError}</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogAction onClick={() => setGitError(null)}>
							OK
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
			<AlertDialog
				open={showDiscardConfirm}
				onOpenChange={(o) => {
					if (!o) setShowDiscardConfirm(false);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Discard All Changes</AlertDialogTitle>
						<AlertDialogDescription>
							Are you sure you want to discard all uncommitted changes? This
							action cannot be undone.
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel onClick={() => setShowDiscardConfirm(false)}>
							Cancel
						</AlertDialogCancel>
						<AlertDialogAction
							variant="destructive"
							onClick={executeDiscardAll}
						>
							Discard All
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
			<AlertDialog
				open={showCreateBranch}
				onOpenChange={(o) => {
					if (!o) setShowCreateBranch(false);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Create Branch</AlertDialogTitle>
						<AlertDialogDescription>
							Enter a name for the new branch.
						</AlertDialogDescription>
					</AlertDialogHeader>
					<Input
						value={newBranchName}
						onChange={(e) => setNewBranchName(e.target.value)}
						placeholder="Branch name"
						autoFocus
						onKeyDown={(e) => {
							if (e.key === "Enter") executeCreateBranch();
						}}
					/>
					<AlertDialogFooter>
						<AlertDialogCancel onClick={() => setShowCreateBranch(false)}>
							Cancel
						</AlertDialogCancel>
						<AlertDialogAction
							onClick={executeCreateBranch}
							disabled={!newBranchName.trim()}
						>
							Create
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}
