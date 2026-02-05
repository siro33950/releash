import { useCallback, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { ActivityBar } from "@/components/layout/ActivityBar";
import { HeaderBar } from "@/components/layout/HeaderBar";
import { StatusBar } from "@/components/layout/StatusBar";
import { ConsolePanel } from "@/components/panels/ConsolePanel";
import { EditorPanel } from "@/components/panels/EditorPanel";
import type { DiffBase, DiffMode } from "@/components/panels/MonacoDiffViewer";
import { SidebarPanel } from "@/components/panels/SidebarPanel";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { UnsavedChangesDialog } from "@/components/panels/UnsavedChangesDialog";
import { useEditorTabs } from "@/hooks/useEditorTabs";
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
	} = useEditorTabs();

	const [diffBase, setDiffBase] = useState<DiffBase>("HEAD");
	const [diffMode, setDiffMode] = useState<DiffMode>("gutter");
	const [closingTabPath, setClosingTabPath] = useState<string | null>(null);

	const handleSave = useCallback(() => {
		if (activeTab?.isDirty) {
			saveFile(activeTab.path);
		}
	}, [activeTab, saveFile]);

	useKeyboardShortcuts({ onSave: handleSave });

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

	const closingTab = closingTabPath
		? tabs.find((t) => t.path === closingTabPath)
		: null;

	return (
		<div className="flex flex-col h-screen w-screen overflow-hidden">
			<HeaderBar
				diffBase={diffBase}
				diffMode={diffMode}
				onDiffBaseChange={setDiffBase}
				onDiffModeChange={setDiffMode}
			/>
			<div className="flex flex-1 overflow-hidden">
				<ActivityBar activeItem="explorer" />
				<Group orientation="horizontal" className="flex-1">
					{/* Sidebar */}
					<Panel
						id="sidebar"
						defaultSize="15"
						minSize={10}
						maxSize="30"
						collapsible={false}
					>
						<SidebarPanel
							onSelectFile={openFile}
							onFileChange={reloadTabIfClean}
							onRename={handleRename}
							onDelete={handleDelete}
						/>
					</Panel>

					<Separator className="w-px bg-border hover:bg-primary/50 cursor-col-resize" />

					{/* Editor + Console */}
					<Panel
						id="editor-console"
						defaultSize="55"
						minSize={20}
						collapsible={false}
					>
						<Group orientation="vertical">
							<Panel
								id="editor"
								defaultSize="70"
								minSize={20}
								collapsible={false}
							>
								<EditorPanel
									tabs={tabs}
									activeTab={activeTab}
									onTabClick={setActiveTab}
									onTabClose={handleTabClose}
									diffMode={diffMode}
									onContentChange={updateTabContent}
								/>
							</Panel>

							<Separator className="h-px bg-border hover:bg-primary/50 cursor-row-resize" />

							<Panel
								id="console"
								defaultSize="30"
								minSize={10}
								collapsible={false}
							>
								<ConsolePanel />
							</Panel>
						</Group>
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
						<TerminalPanel />
					</Panel>
				</Group>
			</div>
			<StatusBar />
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

export default App;
