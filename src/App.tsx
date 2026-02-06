import { loader } from "@monaco-editor/react";
import { open } from "@tauri-apps/plugin-dialog";
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
import { useSettings } from "@/hooks/useSettings";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import { registerDefinitionProviders } from "@/lib/monaco-definition-provider";
import type { LineComment } from "@/types/comment";
import type { DiffBase, DiffMode } from "@/types/settings";

function App() {
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
		closeAllTabs,
		saveAllDirtyTabs,
	} = useEditorTabs();

	const [rootPath, setRootPath] = useState<string | null>(null);
	const [activeView, setActiveView] = useState<string>("explorer");
	const { branch } = useCurrentBranch(rootPath);
	const {
		settings,
		updateTheme,
		updateFontSize,
		updateDefaultDiffBase,
		updateDefaultDiffMode,
	} = useSettings();
	const { comments, addComment, markAsSent } = useLineComments();
	const { stageHunk, unstageHunk } = useGitActions();
	const terminalRef = useRef<TerminalPanelHandle>(null);
	const [gitRefreshKey, setGitRefreshKey] = useState(0);
	const refreshGit = useCallback(() => setGitRefreshKey((k) => k + 1), []);

	const rootPathRef = useRef(rootPath);
	rootPathRef.current = rootPath;

	useEffect(() => {
		loader.init().then((monaco) => {
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
		});
	}, [openFile]);

	const [diffBase, setDiffBase] = useState<DiffBase>(settings.defaultDiffBase);
	const [diffMode, setDiffMode] = useState<DiffMode>(settings.defaultDiffMode);
	const [closingTabPath, setClosingTabPath] = useState<string | null>(null);
	const [pendingRootPath, setPendingRootPath] = useState<string | null>(null);
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
			if (!rootPath) return;
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

	const handleOpenFolder = useCallback(async () => {
		const selected = await open({ directory: true });
		if (!selected) return;

		const hasDirty = tabs.some((t) => t.isDirty);
		if (hasDirty) {
			setPendingRootPath(selected);
		} else {
			setRootPath(selected);
			closeAllTabs();
		}
	}, [tabs, closeAllTabs]);

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

	const handleFolderChangeSave = useCallback(async () => {
		if (!pendingRootPath) return;
		await saveAllDirtyTabs();
		setRootPath(pendingRootPath);
		closeAllTabs();
		setPendingRootPath(null);
	}, [pendingRootPath, saveAllDirtyTabs, closeAllTabs]);

	const handleFolderChangeDiscard = useCallback(() => {
		if (!pendingRootPath) return;
		setRootPath(pendingRootPath);
		closeAllTabs();
		setPendingRootPath(null);
	}, [pendingRootPath, closeAllTabs]);

	const handleFolderChangeCancel = useCallback(() => {
		setPendingRootPath(null);
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

	const dirtyTabCount = tabs.filter((t) => t.isDirty).length;
	const showUnsavedDialog = !!closingTabPath || !!pendingRootPath;
	const unsavedFileName = closingTab
		? closingTab.name
		: `${dirtyTabCount}個のファイル`;
	const unsavedOnSave = closingTabPath
		? handleUnsavedSave
		: handleFolderChangeSave;
	const unsavedOnDiscard = closingTabPath
		? handleUnsavedDiscard
		: handleFolderChangeDiscard;
	const unsavedOnCancel = closingTabPath
		? handleUnsavedCancel
		: handleFolderChangeCancel;

	return (
		<div className="flex flex-col h-screen w-screen overflow-hidden">
			<div className="flex flex-1 overflow-hidden">
				<ActivityBar activeItem={activeView} onItemClick={setActiveView} />
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
								onThemeChange={updateTheme}
								onFontSizeChange={updateFontSize}
								onDiffBaseChange={updateDefaultDiffBase}
								onDiffModeChange={updateDefaultDiffMode}
							/>
						) : (
							<SidebarPanel
								rootPath={rootPath}
								onOpenFolder={handleOpenFolder}
								onSelectFile={openFile}
								onFileChange={reloadTabIfClean}
								onRename={handleRename}
								onDelete={handleDelete}
							/>
						)}
					</Panel>

					<Separator className="w-px bg-border hover:bg-primary/50 cursor-col-resize" />

					{/* Editor */}
					<Panel id="editor" defaultSize="55" minSize={20} collapsible={false}>
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
						/>
					</Panel>
				</Group>
			</div>
			<StatusBar
				branch={branch ?? undefined}
				language={activeTab?.language}
				encoding={activeTab ? "UTF-8" : undefined}
				eol={activeTab?.eol}
			/>
			<UnsavedChangesDialog
				open={showUnsavedDialog}
				fileName={unsavedFileName}
				onSave={unsavedOnSave}
				onDiscard={unsavedOnDiscard}
				onCancel={unsavedOnCancel}
			/>
		</div>
	);
}

export default App;
