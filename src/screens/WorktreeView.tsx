import { loader } from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Loader2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { ActivityBar } from "@/components/layout/ActivityBar";
import { StatusBar } from "@/components/layout/StatusBar";
import { EditorPanel } from "@/components/panels/EditorPanel";
import { SearchPanel } from "@/components/panels/SearchPanel";
import { SettingsPanel } from "@/components/panels/SettingsPanel";
import { SidebarPanel } from "@/components/panels/SidebarPanel";
import { SourceControlPanel } from "@/components/panels/SourceControlPanel";
import {
	TerminalPanel,
	type TerminalPanelHandle,
} from "@/components/panels/TerminalPanel";
import { UnsavedChangesDialog } from "@/components/panels/UnsavedChangesDialog";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useEditorTabs } from "@/hooks/useEditorTabs";
import { useGitActions } from "@/hooks/useGitActions";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";
import { useLineComments } from "@/hooks/useLineComments";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import { registerDefinitionProviders } from "@/lib/monaco-definition-provider";
import type { LineComment } from "@/types/comment";
import type { AgentState, AgentStateSync } from "@/types/protocol";
import {
	buildTerminalCommand,
	type AppSettings,
	type DiffBase,
	type DiffMode,
} from "@/types/settings";

interface WorktreeViewProps {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	onGoHome: () => void;
}

export function WorktreeView({
	rootPath,
	settings,
	onSettingsSave,
	onGoHome,
}: WorktreeViewProps) {
	const {
		tabs,
		activeTab,
		openFile,
		closeTab,
		setActiveTab,
		reloadTabIfClean,
		updateTabContent,
		saveFile,
		updateTabPath,
		closeTabsByPrefix,
	} = useEditorTabs();

	const [activeView, setActiveView] = useState<string>("explorer");
	const { branch } = useCurrentBranch(rootPath);
	const [ready, setReady] = useState(false);
	const [agentState, setAgentState] = useState<AgentState | undefined>();
	const { comments, addComment, markAsSent } = useLineComments();
	const { stageHunk, unstageHunk } = useGitActions();
	const terminalRef = useRef<TerminalPanelHandle>(null);
	const [gitRefreshKey, setGitRefreshKey] = useState(0);
	const refreshGit = useCallback(() => setGitRefreshKey((k) => k + 1), []);

	useEffect(() => {
		if (branch != null) {
			setReady(true);
		}
	}, [branch]);

	useEffect(() => {
		const unlisten = listen<AgentStateSync>("agent-state-changed", (event) => {
			if (event.payload.worktree_path === rootPath) {
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

		const unlistenConnected = listen("remote-connected", () => {
			broadcastComments(commentsRef.current);
		});

		return () => {
			unlistenComment.then((f) => f());
			unlistenConnected.then((f) => f());
		};
	}, [addComment, broadcastComments]);

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
						openFile(absolutePath);
						setPendingReveal({ path: absolutePath, line });
					},
					getRootPath: () => rootPathRef.current,
				});
			})
			.catch((error) => {
				console.error("Failed to initialize Monaco:", error);
			});
	}, [openFile]);

	const [diffBase, setDiffBase] = useState<DiffBase>(settings.defaultDiffBase);
	const [diffMode, setDiffMode] = useState<DiffMode>(settings.defaultDiffMode);
	const [closingTabPath, setClosingTabPath] = useState<string | null>(null);
	const [pendingReveal, setPendingReveal] = useState<{
		path: string;
		line: number;
	} | null>(null);
	const [searchFocusKey, setSearchFocusKey] = useState(0);

	const handleSave = useCallback(() => {
		if (activeTab?.isDirty) {
			saveFile(activeTab.path);
		}
	}, [activeTab, saveFile]);

	const handleSearch = useCallback(() => {
		setActiveView("search");
		setSearchFocusKey((k) => k + 1);
	}, []);

	useKeyboardShortcuts({ onSave: handleSave, onSearch: handleSearch });

	const handleSearchResultClick = useCallback(
		(relativePath: string, line: number) => {
			const absolutePath = `${rootPath}/${relativePath}`;
			openFile(absolutePath);
			setPendingReveal({ path: absolutePath, line });
		},
		[rootPath, openFile],
	);

	const handleSearchOccurrences = useCallback((_text: string) => {
		setActiveView("search");
		setSearchFocusKey((k) => k + 1);
	}, []);

	const handleTabClose = useCallback(
		(path: string) => {
			const tab = tabs.find((t) => t.path === path);
			if (tab?.isDirty) {
				setClosingTabPath(path);
			} else {
				closeTab(path);
			}
		},
		[tabs, closeTab],
	);

	const handleUnsavedSave = useCallback(async () => {
		if (!closingTabPath) return;
		try {
			await saveFile(closingTabPath);
			closeTab(closingTabPath);
			setClosingTabPath(null);
		} catch (e) {
			console.error("Failed to save file:", e);
			setClosingTabPath(null);
		}
	}, [closingTabPath, saveFile, closeTab]);

	const handleUnsavedDiscard = useCallback(() => {
		if (!closingTabPath) return;
		closeTab(closingTabPath);
		setClosingTabPath(null);
	}, [closingTabPath, closeTab]);

	const handleUnsavedCancel = useCallback(() => {
		setClosingTabPath(null);
	}, []);

	const handleRename = useCallback(
		(oldPath: string, newPath: string) => {
			updateTabPath(oldPath, newPath);
		},
		[updateTabPath],
	);

	const handleDelete = useCallback(
		(path: string) => {
			closeTabsByPrefix(path);
		},
		[closeTabsByPrefix],
	);

	const handleSendToTerminal = useCallback(
		(unsent: LineComment[]) => {
			const text = formatCommentsForTerminal(unsent);
			if (text && terminalRef.current) {
				terminalRef.current.writeToTerminal(`${text}\n`);
				markAsSent(unsent.map((c) => c.id));
			}
		},
		[markAsSent],
	);

	const closingTab = closingTabPath
		? tabs.find((t) => t.path === closingTabPath)
		: null;

	return (
		<div className="flex flex-col h-screen w-screen overflow-hidden bg-background text-foreground">
			<div className="flex flex-1 overflow-hidden">
				<ActivityBar
					activeItem={activeView}
					onItemClick={setActiveView}
					onGoHome={onGoHome}
				/>
				{!ready ? (
					<div className="flex-1 flex items-center justify-center">
						<Loader2 className="size-6 text-muted-foreground animate-spin" />
					</div>
				) : (
					<Group orientation="horizontal" className="flex-1">
						{/* Sidebar */}
						<Panel
							id="sidebar"
							defaultSize="15"
							minSize={10}
							maxSize="30"
							collapsible={false}
						>
							{activeView === "git" ? (
								<SourceControlPanel
									rootPath={rootPath}
									onSelectFile={openFile}
									onGitChanged={refreshGit}
									gitRefreshKey={gitRefreshKey}
								/>
							) : activeView === "search" ? (
								<SearchPanel
									rootPath={rootPath}
									onSelectFileAtLine={handleSearchResultClick}
									focusKey={searchFocusKey}
								/>
							) : activeView === "settings" ? (
								<SettingsPanel
									settings={settings}
									onSave={onSettingsSave}
								/>
							) : (
								<SidebarPanel
									rootPath={rootPath}
									onOpenFolder={onGoHome}
									onSelectFile={openFile}
									onFileChange={reloadTabIfClean}
									onRename={handleRename}
									onDelete={handleDelete}
								/>
							)}
						</Panel>

						<Separator className="w-px bg-border hover:bg-primary/50 cursor-col-resize" />

						{/* Editor */}
						<Panel
							id="editor"
							defaultSize="55"
							minSize={20}
							collapsible={false}
						>
							<EditorPanel
								tabs={tabs}
								activeTab={activeTab}
								onTabClick={setActiveTab}
								onTabClose={handleTabClose}
								diffBase={diffBase}
								diffMode={diffMode}
								onDiffBaseChange={setDiffBase}
								onDiffModeChange={setDiffMode}
								onContentChange={updateTabContent}
								fontSize={settings.fontSize}
								comments={comments}
								onAddComment={addComment}
								rootPath={rootPath}
								onStageHunk={stageHunk}
								onUnstageHunk={unstageHunk}
								onSendToTerminal={handleSendToTerminal}
								theme={settings.theme}
								gitRefreshKey={gitRefreshKey}
								onGitChanged={refreshGit}
								externalRevealLine={pendingReveal}
								onExternalRevealConsumed={() => setPendingReveal(null)}
								onSearchOccurrences={handleSearchOccurrences}
							/>
						</Panel>

						<Separator className="w-px bg-border hover:bg-primary/50 cursor-col-resize" />

						{/* Terminal */}
						<Panel
							id="terminal"
							defaultSize="30"
							minSize={10}
							maxSize="60"
							collapsible={false}
						>
							<TerminalPanel
								ref={terminalRef}
								key={rootPath}
								cwd={rootPath}
								theme={settings.theme}
								terminalStartupCommand={buildTerminalCommand(settings)}
							/>
						</Panel>
					</Group>
				)}
			</div>
			<StatusBar
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
		</div>
	);
}
