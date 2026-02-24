import { FileIcon } from "@react-symbols/icons/utils";
import { Bot, PanelLeft, PanelRight, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	Group,
	Panel,
	type PanelImperativeHandle,
	type PanelSize,
	Separator,
} from "react-resizable-panels";
import { StatusBar } from "@/components/layout/StatusBar";
import { type TogglePanel, ViewToolbar } from "@/components/layout/ViewToolbar";
import { AgentTab } from "@/components/panels/AgentTab";
import { EditorTabContent } from "@/components/panels/EditorTabContent";
import { EmptyState } from "@/components/panels/EmptyState";
import { PullRequestPanel } from "@/components/panels/PullRequestPanel";
import { RightSidebarBottom } from "@/components/panels/RightSidebarBottom";
import {
	RightSidebarTop,
	type RightTopTab,
} from "@/components/panels/RightSidebarTop";
import { SearchPanel } from "@/components/panels/SearchPanel";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { SourceControlPanel } from "@/components/panels/SourceControlPanel";
import { UnsavedChangesDialog } from "@/components/panels/UnsavedChangesDialog";
import {
	DraggableTabs,
	SortableTabTrigger,
} from "@/components/ui/draggable-tabs";
import { Tabs, TabsContent, TabsList } from "@/components/ui/tabs";
import { EditorContext } from "@/contexts/EditorContext";
import { useWorktreeState } from "@/screens/useWorktreeState";
import {
	CreateBranchDialog,
	DiscardAllDialog,
	GitErrorDialog,
	SavingConflictDialog,
} from "@/screens/WorktreeViewDialogs";
import type { AgentState } from "@/types/protocol";
import type { AppSettings } from "@/types/settings";
import { buildTerminalCommand } from "@/types/settings";

interface StatusBarData {
	branch?: string;
	language?: string;
	encoding?: string;
	eol?: "LF" | "CRLF";
	agentState?: AgentState;
}

interface MainLayoutProps {
	selectedRootPath: string | null;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	leftNav: React.ReactNode;
}

function WorktreeContent({
	rootPath,
	settings,
	onSettingsSave,
	onStatusChange,
	rightPanelRef,
	onRightResize,
}: {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	onStatusChange: (data: StatusBarData) => void;
	rightPanelRef: React.Ref<PanelImperativeHandle | null>;
	onRightResize: (size: PanelSize) => void;
}) {
	const s = useWorktreeState({
		rootPath,
		settings,
		onSettingsSave,
		isActive: true,
	});

	useEffect(() => {
		onStatusChange({
			branch: s.branch ?? undefined,
			language: s.activeTab?.language,
			encoding: s.activeTab ? "UTF-8" : undefined,
			eol: s.activeTab?.eol,
			agentState: s.agentState,
		});
	}, [s.branch, s.activeTab, s.agentState, onStatusChange]);

	const handleTabSelect = useCallback(
		(tabId: string) => {
			s.editorLayout.selectTabById(tabId);
		},
		[s.editorLayout],
	);

	return (
		<EditorContext.Provider value={s.editorContextValue}>
			{/* Center */}
			<Panel id="center" minSize="30%">
				<div
					ref={s.editorDropZoneRef}
					role="application"
					className="h-full relative overflow-hidden flex flex-col"
					onDragOver={s.handleEditorDragOver}
					onDragLeave={s.handleEditorDragLeave}
					onDrop={s.handleEditorDrop}
				>
					<Tabs
						value={s.editorLayout.activeTabId}
						onValueChange={(val) => {
							handleTabSelect(val);
						}}
						className="flex flex-col h-full gap-0"
					>
						<DraggableTabs
							items={s.editorLayout.tabs}
							onReorder={s.editorLayout.reorderTabs}
						>
							<TabsList variant="line">
								{s.editorLayout.tabs.map((tab) => (
									<SortableTabTrigger
										key={tab.id}
										id={tab.id}
										value={tab.id}
										disabled={!tab.draggable}
										className="gap-2"
									>
										{tab.component === "agent" ? (
											<Bot className="h-4 w-4" />
										) : (
											<FileIcon fileName={tab.name} className="h-4 w-4" />
										)}
										<span>{tab.name}</span>
										{tab.isDirty && (
											<span className="w-2 h-2 rounded-full bg-foreground shrink-0" />
										)}
										{tab.closable && (
											// biome-ignore lint/a11y/useSemanticElements: nested inside TabsTrigger <button>, cannot use <button>
											<span
												role="button"
												tabIndex={0}
												className="p-0.5 rounded hover:bg-muted-foreground/20 transition-colors shrink-0"
												aria-label={`Close ${tab.name}`}
												onClick={(e) => {
													e.stopPropagation();
													if (tab.path) s.editorLayout.closeTab(tab.path);
												}}
												onKeyDown={(e) => {
													if (e.key === "Enter" || e.key === " ") {
														e.preventDefault();
														e.stopPropagation();
														if (tab.path) s.editorLayout.closeTab(tab.path);
													}
												}}
											>
												<X className="size-3.5" />
											</span>
										)}
									</SortableTabTrigger>
								))}
							</TabsList>
						</DraggableTabs>
						<div className="flex-1 relative" style={{ minHeight: 0 }}>
							{s.editorLayout.tabs.map((tab) => (
								<TabsContent
									key={tab.id}
									value={tab.id}
									forceMount
									className="absolute inset-0 isolate m-0 data-[state=inactive]:hidden"
								>
									{tab.component === "agent" ? (
										<AgentTab
											ref={s.terminalRef}
											rootPath={rootPath}
											theme={settings.theme}
											terminalStartupCommand={buildTerminalCommand(settings)}
											agentType={settings.agent}
										/>
									) : tab.path ? (
										<EditorTabContent
											key={tab.path}
											filePath={tab.path}
											externalRevealLine={s.pendingReveal}
											onExternalRevealConsumed={() =>
												s.dispatchEditor({
													type: "SET_PENDING_REVEAL",
													reveal: null,
												})
											}
										/>
									) : null}
								</TabsContent>
							))}
						</div>
					</Tabs>
					{s.editorDragOver && (
						<div className="absolute inset-0 flex items-center justify-center bg-primary/10 border-2 border-dashed border-primary rounded pointer-events-none">
							<span className="text-sm font-medium text-primary bg-background/80 px-3 py-1.5 rounded">
								Drop to open file
							</span>
						</div>
					)}
				</div>
			</Panel>
			<Separator />
			{/* Right Sidebar */}
			<Panel
				id="right"
				panelRef={rightPanelRef}
				defaultSize="20%"
				minSize="10%"
				collapsible
				collapsedSize="0%"
				onResize={onRightResize}
			>
				<Group orientation="vertical">
					<Panel id="right-top" defaultSize="50%" minSize="20%">
						<div className="h-full overflow-hidden border-l border-border">
							<RightSidebarTop
								activeTab={
									s.activeView === "git"
										? "changes"
										: s.activeView === "search"
											? "search"
											: s.activeView === "pr"
												? "pr"
												: "explorer"
								}
								onTabChange={(tab: RightTopTab) => {
									const view = tab === "changes" ? "git" : tab;
									s.dispatchEditor({
										type: "SET_ACTIVE_VIEW",
										view,
									});
								}}
								explorerContent={s.sidebarContent}
								changesContent={
									<SourceControlPanel
										rootPath={rootPath}
										onSelectFile={s.handleOpenFile}
										onGitChanged={s.refreshGit}
										gitRefreshKey={s.gitRefreshKey}
									/>
								}
								searchContent={
									<SearchPanel
										rootPath={rootPath}
										onSelectFileAtLine={s.handleSearchResultClick}
										focusKey={s.searchFocusKey}
										initialQuery={s.searchInitialQuery}
									/>
								}
								prContent={
									<PullRequestPanel rootPath={rootPath} branch={s.branch} />
								}
							/>
						</div>
					</Panel>
					<Separator />
					<Panel id="right-bottom" defaultSize="50%" minSize="20%">
						<div
							data-testid="review"
							className="h-full overflow-hidden border-l border-t border-border"
						>
							<RightSidebarBottom
								rootPath={rootPath}
								theme={settings.theme}
								comments={s.comments}
								onCommentClick={s.handleCommentClick}
								onDeleteComment={s.removeComment}
								onUpdateComment={s.updateComment}
								onSendToTerminal={s.handleSendToTerminal}
								onSendComment={s.handleSendComment}
								onCopyComment={s.handleCopyComment}
								showSentComments={s.showSentComments}
								onToggleShowSent={s.toggleShowSentComments}
							/>
						</div>
					</Panel>
				</Group>
			</Panel>

			{/* Dialogs */}
			<UnsavedChangesDialog
				open={!!s.closingTabPath}
				fileName={s.closingTab?.name ?? ""}
				onSave={s.handleUnsavedSave}
				onDiscard={s.handleUnsavedDiscard}
				onCancel={s.handleUnsavedCancel}
			/>
			<SavingConflictDialog
				open={!!s.savingConflictPath}
				onOpenChange={(o) => {
					if (!o) s.dispatchUI({ type: "SET_SAVING_CONFLICT", path: null });
				}}
				onOverwrite={() => {
					if (s.savingConflictPath) {
						s.clearExternalChange(s.savingConflictPath);
						s.saveFile(s.savingConflictPath);
					}
					s.dispatchUI({ type: "SET_SAVING_CONFLICT", path: null });
				}}
			/>
			<GitErrorDialog
				error={s.gitError}
				onOpenChange={(o) => {
					if (!o) s.dispatchGit({ type: "SET_GIT_ERROR", error: null });
				}}
				onDismiss={() => s.dispatchGit({ type: "SET_GIT_ERROR", error: null })}
			/>
			<DiscardAllDialog
				open={s.showDiscardConfirm}
				onOpenChange={(o) => {
					if (!o) s.dispatchUI({ type: "SET_DISCARD_CONFIRM", show: false });
				}}
				onDiscard={s.gitActions.executeDiscardAll}
			/>
			<CreateBranchDialog
				open={s.showCreateBranch}
				onOpenChange={(o) => {
					if (!o) s.dispatchUI({ type: "CLOSE_CREATE_BRANCH" });
				}}
				branchName={s.newBranchName}
				onBranchNameChange={(name) =>
					s.dispatchUI({ type: "SET_NEW_BRANCH_NAME", name })
				}
				onCreate={s.gitActions.executeCreateBranch}
			/>
			<SettingsModal
				open={s.isSettingsOpen}
				onOpenChange={(open) =>
					s.dispatchUI({ type: "SET_SETTINGS_OPEN", open })
				}
				settings={settings}
				onSave={onSettingsSave}
			/>
		</EditorContext.Provider>
	);
}

export function MainLayout({
	selectedRootPath,
	settings,
	onSettingsSave,
	leftNav,
}: MainLayoutProps) {
	const [statusBarData, setStatusBarData] = useState<StatusBarData>({});

	const handleStatusChange = useCallback((data: StatusBarData) => {
		setStatusBarData(data);
	}, []);

	const leftNavRef = useRef<PanelImperativeHandle>(null);
	const rightPanelRef = useRef<PanelImperativeHandle>(null);

	const [leftNavVisible, setLeftNavVisible] = useState(true);
	const [rightVisible, setRightVisible] = useState(true);

	const handleLeftNavResize = useCallback((size: PanelSize) => {
		const visible = size.asPercentage > 0;
		setLeftNavVisible((prev) => (prev === visible ? prev : visible));
	}, []);
	const handleRightResize = useCallback((size: PanelSize) => {
		const visible = size.asPercentage > 0;
		setRightVisible((prev) => (prev === visible ? prev : visible));
	}, []);

	const togglePanels = useMemo<TogglePanel[]>(
		() => [
			{
				id: "left-nav",
				icon: PanelLeft,
				label: "Sidebar",
				visible: leftNavVisible,
				onToggle: () => {
					const panel = leftNavRef.current;
					if (!panel) return;
					panel.isCollapsed() ? panel.expand() : panel.collapse();
				},
			},
			{
				id: "right",
				icon: PanelRight,
				label: "Right Sidebar",
				visible: rightVisible,
				onToggle: () => {
					const panel = rightPanelRef.current;
					if (!panel) return;
					panel.isCollapsed() ? panel.expand() : panel.collapse();
				},
			},
		],
		[leftNavVisible, rightVisible],
	);

	return (
		<div className="flex flex-col h-screen w-screen overflow-hidden bg-background text-foreground">
			<ViewToolbar panels={togglePanels} />
			<div className="flex-1 overflow-hidden">
				<Group orientation="horizontal" className="h-full">
					<Panel
						id="left-nav"
						panelRef={leftNavRef}
						defaultSize={230}
						minSize={230}
						collapsible
						collapsedSize="0%"
						onResize={handleLeftNavResize}
					>
						<div className="h-full overflow-hidden border-r border-border">
							{leftNav}
						</div>
					</Panel>
					<Separator />
					{selectedRootPath ? (
						<WorktreeContent
							key={selectedRootPath}
							rootPath={selectedRootPath}
							settings={settings}
							onSettingsSave={onSettingsSave}
							onStatusChange={handleStatusChange}
							rightPanelRef={rightPanelRef}
							onRightResize={handleRightResize}
						/>
					) : (
						<Panel id="center" minSize="30%">
							<EmptyState
								title="No worktree selected"
								description="Select a worktree from the sidebar to start working"
							/>
						</Panel>
					)}
				</Group>
			</div>
			<StatusBar
				className="shrink-0"
				branch={selectedRootPath ? statusBarData.branch : undefined}
				language={selectedRootPath ? statusBarData.language : undefined}
				encoding={selectedRootPath ? statusBarData.encoding : undefined}
				eol={selectedRootPath ? statusBarData.eol : undefined}
				agentState={selectedRootPath ? statusBarData.agentState : undefined}
			/>
		</div>
	);
}
