import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { ActivityBar } from "@/components/layout/ActivityBar";
import { StatusBar } from "@/components/layout/StatusBar";
import { EditorPanel } from "@/components/panels/EditorPanel";
import type { DiffBase, DiffMode } from "@/components/panels/MonacoDiffViewer";
import { SidebarPanel } from "@/components/panels/SidebarPanel";
import { SourceControlPanel } from "@/components/panels/SourceControlPanel";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { UnsavedChangesDialog } from "@/components/panels/UnsavedChangesDialog";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useEditorTabs } from "@/hooks/useEditorTabs";
import { useGitStatus } from "@/hooks/useGitStatus";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";

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
	const { changedFiles } = useGitStatus(rootPath);
	const [diffBase, setDiffBase] = useState<DiffBase>("HEAD");
	const [diffMode, setDiffMode] = useState<DiffMode>("gutter");
	const [closingTabPath, setClosingTabPath] = useState<string | null>(null);
	const [pendingRootPath, setPendingRootPath] = useState<string | null>(null);

	const handleSave = useCallback(() => {
		if (activeTab?.isDirty) {
			saveFile(activeTab.path);
		}
	}, [activeTab, saveFile]);

	useKeyboardShortcuts({ onSave: handleSave });

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
						<TerminalPanel key={rootPath} cwd={rootPath} />
					</Panel>
				</Group>
			</div>
			<StatusBar
			branch={branch ?? undefined}
			status={changedFiles.length > 0 ? `${changedFiles.length} changed` : "Clean"}
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
