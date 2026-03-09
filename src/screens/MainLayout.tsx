import { FileIcon } from "@react-symbols/icons/utils";
import { PanelLeft, PanelRight, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	Group,
	Panel,
	type PanelImperativeHandle,
	type PanelSize,
	Separator,
} from "react-resizable-panels";
import { BranchSelector } from "@/components/layout/BranchSelector";

import { type TogglePanel, ViewToolbar } from "@/components/layout/ViewToolbar";
import { EditorTabContent } from "@/components/panels/EditorTabContent";
import { EmptyState } from "@/components/panels/EmptyState";
import { PostToPrPreview } from "@/components/panels/PostToPrPreview";
import { PullRequestPanel } from "@/components/panels/PullRequestPanel";
import {
	type RightBottomTab,
	RightSidebarBottom,
} from "@/components/panels/RightSidebarBottom";
import {
	RightSidebarTop,
	type RightTopTab,
} from "@/components/panels/RightSidebarTop";
import { SearchPanel } from "@/components/panels/SearchPanel";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { SourceControlPanel } from "@/components/panels/SourceControlPanel";
import { SymbolOutlinePanel } from "@/components/panels/SymbolOutlinePanel";
import { ThreadAIModal } from "@/components/panels/ThreadAIModal";
import { UnsavedChangesDialog } from "@/components/panels/UnsavedChangesDialog";
import { WorkflowView } from "@/components/panels/WorkflowView";
import { Button } from "@/components/ui/button";
import {
	DraggableTabs,
	SortableTabTrigger,
} from "@/components/ui/draggable-tabs";
import { Tabs, TabsContent, TabsList } from "@/components/ui/tabs";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { EditorContext } from "@/contexts/EditorContext";
import { GitStatusProvider } from "@/contexts/GitStatusContext";
import { useBaseBranch } from "@/hooks/useBaseBranch";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useWorkspacePersistence } from "@/hooks/useWorkspacePersistence";
import { cn } from "@/lib/utils";
import {
	type InternalWorktreeState,
	useWorktreeState,
} from "@/screens/useWorktreeState";
import {
	CreateBranchDialog,
	DiscardAllDialog,
	GitErrorDialog,
	SavingConflictDialog,
} from "@/screens/WorktreeViewDialogs";
import type { AppSettings } from "@/types/settings";
import { buildTerminalCommand } from "@/types/settings";
import type { Thread } from "@/types/thread";
import type { WorkspaceState } from "@/types/workspace-state";

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
	rightPanelRef,
	onRightResize,
	onSwitchToEditor,
	centerTab,
	setCenterTab,
	leftPanels,
	branchSelector,
	togglePanels,
	initialWorkspaceState,
	internalStateMapRef,
}: {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	rightPanelRef: React.Ref<PanelImperativeHandle | null>;
	onRightResize: (size: PanelSize) => void;
	onSwitchToEditor: () => void;
	centerTab: string;
	setCenterTab: (tab: string) => void;
	leftPanels?: TogglePanel[];
	branchSelector: React.ReactNode;
	togglePanels: TogglePanel[];
	initialWorkspaceState?: WorkspaceState;
	internalStateMapRef: React.MutableRefObject<
		Map<string, InternalWorktreeState>
	>;
}) {
	const centerTabRef = useRef(centerTab);
	centerTabRef.current = centerTab;

	const workflowRightRef = useRef<PanelImperativeHandle>(null);

	// rightVisible（togglePanels[0].visible）を状態源として両パネルを同期
	// biome-ignore lint/correctness/useExhaustiveDependencies: centerTab 変更時にも再実行し、display:none から復帰したパネルを同期する
	useEffect(() => {
		const shouldBeVisible = togglePanels[0]?.visible ?? true;
		const panels = [
			typeof rightPanelRef === "object" ? rightPanelRef?.current : null,
			workflowRightRef.current,
		];
		requestAnimationFrame(() => {
			for (const panel of panels) {
				if (!panel) continue;
				if (shouldBeVisible && panel.isCollapsed()) panel.expand();
				else if (!shouldBeVisible && !panel.isCollapsed()) panel.collapse();
			}
		});
	}, [togglePanels, rightPanelRef, centerTab]);

	const rightBottomRef = useRef<PanelImperativeHandle>(null);

	const handleToggleRightBottom = useCallback(() => {
		const panel = rightBottomRef.current;
		if (!panel) return;
		panel.isCollapsed() ? panel.expand() : panel.collapse();
	}, []);

	// Restore rightBottom collapsed state from initialWorkspaceState
	const initialRightBottomCollapsed =
		initialWorkspaceState?.layout.rightBottomCollapsed ?? false;
	useEffect(() => {
		if (!initialRightBottomCollapsed) return;
		const panel = rightBottomRef.current;
		if (!panel) return;
		requestAnimationFrame(() => {
			panel.collapse();
		});
	}, [initialRightBottomCollapsed]);

	const s = useWorktreeState({
		rootPath,
		settings,
		onSettingsSave,
		isActive: true,
		centerTabRef,
		onSwitchToEditor,
		initialWorkspaceState,
		internalStateMapRef,
	});

	const {
		handleThreadClick: baseThreadClick,
		handleSendToTerminal: baseSendToTerminal,
	} = s;

	// スレッドクリック → Editorに切り替え + 既存の行ジャンプ
	const handleThreadClick = useCallback(
		(filePath: string, lineNumber: number) => {
			setCenterTab("editor");
			baseThreadClick(filePath, lineNumber);
		},
		[setCenterTab, baseThreadClick],
	);

	// スレッド一括送信 → 既存処理 + Workflowに切り替え
	const handleThreadsSent = useCallback(
		(threadsToSend: Thread[]) => {
			baseSendToTerminal(threadsToSend);
			setCenterTab("workflow");
		},
		[setCenterTab, baseSendToTerminal],
	);

	// TODO: planDocument will become dynamic state when workflow execution is implemented.
	// When it does, add planDocument to the deps of handleCreateDocumentThread and
	// the recalculation effect below.
	const planDocument = "";

	const handleCreateDocumentThread = useCallback(
		(line: number) => {
			s.createThread(
				"workflow://plan",
				line,
				"",
				undefined,
				undefined,
				undefined,
				undefined,
				planDocument || undefined,
			);
		},
		[s.createThread],
	);

	// Recalculate thread anchors when the plan document content changes
	useEffect(() => {
		if (planDocument) {
			s.recalculateAnchorsForFile("workflow://plan", planDocument);
		}
	}, [s.recalculateAnchorsForFile]);

	const handlePrFileSelect = useCallback(
		(filename: string, _orig: string, _mod: string) => {
			const absolutePath = `${rootPath}/${filename}`;
			s.handleOpenFile(absolutePath);
		},
		[rootPath, s.handleOpenFile],
	);

	const handleTabSelect = useCallback(
		(tabId: string) => {
			s.editorLayout.selectTabById(tabId);
		},
		[s.editorLayout],
	);

	// ファイルタブが開かれたら自動でEditorビューに切り替え
	// 初回マウント時はスキップ（復元されたcenterTabを上書きしないため）
	const initialMountRef = useRef(true);
	useEffect(() => {
		if (initialMountRef.current) {
			initialMountRef.current = false;
			return;
		}
		if (s.editorLayout.activeTabId !== "") {
			onSwitchToEditor();
		}
	}, [s.editorLayout.activeTabId, onSwitchToEditor]);

	return (
		<GitStatusProvider rootPath={rootPath} externalRefreshKey={s.gitRefreshKey}>
			<EditorContext.Provider value={s.editorContextValue}>
				<div className="flex flex-col h-full overflow-hidden">
					<ViewToolbar
						leftPanels={leftPanels}
						rightPanels={togglePanels}
						rightSlot={branchSelector}
					/>
					<div className="flex-1 overflow-hidden">
						{/* Workflow view */}
						<TabsContent
							value="workflow"
							forceMount
							className="h-full m-0 data-[state=inactive]:hidden"
						>
							<WorkflowView
								ref={s.terminalRef}
								rootPath={rootPath}
								theme={settings.theme}
								terminalStartupCommand={buildTerminalCommand(settings)}
								agentType={settings.agent}
								planDocument={planDocument}
								phase="requirements"
								planTimeline={[]}
								implTimeline={[]}
								threads={s.threads}
								onThreadClick={handleThreadClick}
								rightPanelRef={workflowRightRef}
								onRightPanelResize={onRightResize}
								onDeleteThread={s.removeThread}
								onResolveThread={s.resolveThread}
								onCreateDocumentThread={handleCreateDocumentThread}
								initialDocTerminalRatio={s.workflowPanelRatios}
								onDocTerminalResize={s.setWorkflowPanelRatios}
							/>
						</TabsContent>
						{/* Editor view */}
						<TabsContent
							value="editor"
							forceMount
							className="h-full m-0 data-[state=inactive]:hidden"
						>
							<Group orientation="horizontal" className="h-full">
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
												<TabsList
													variant="line"
													className="w-auto max-w-full overflow-x-auto overflow-y-hidden justify-start [&::-webkit-scrollbar]:hidden [scrollbar-width:none]"
												>
													{s.editorLayout.tabs.map((tab) => (
														<SortableTabTrigger
															key={tab.id}
															id={tab.id}
															value={tab.id}
															disabled={!tab.draggable}
															className="gap-2 flex-none"
														>
															<FileIcon
																fileName={tab.name}
																className="h-4 w-4"
															/>
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
																	onPointerDown={(e) => {
																		e.stopPropagation();
																	}}
																	onMouseDown={(e) => {
																		e.stopPropagation();
																	}}
																	onClick={(e) => {
																		e.stopPropagation();
																		if (tab.path)
																			s.editorLayout.closeTab(tab.path);
																	}}
																	onKeyDown={(e) => {
																		if (e.key === "Enter" || e.key === " ") {
																			e.preventDefault();
																			e.stopPropagation();
																			if (tab.path)
																				s.editorLayout.closeTab(tab.path);
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
												{s.editorLayout.tabs.map((tab) =>
													tab.path ? (
														<TabsContent
															key={tab.id}
															value={tab.id}
															forceMount
															className="absolute inset-0 isolate m-0 data-[state=inactive]:hidden"
														>
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
														</TabsContent>
													) : null,
												)}
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
									defaultSize={280}
									minSize={280}
									collapsible
									collapsedSize="0%"
									onResize={onRightResize}
								>
									<div className="flex flex-col h-full border-l border-border">
										<div className="flex-1 overflow-hidden">
											<Group orientation="vertical">
												<Panel id="right-top" defaultSize="50%" minSize="20%">
													<div className="h-full overflow-hidden">
														<RightSidebarTop
															activeTab={
																s.activeView === "git"
																	? "changes"
																	: s.activeView === "search"
																		? "search"
																		: s.activeView === "pr"
																			? "pr"
																			: s.activeView === "symbols"
																				? "symbols"
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
																<PullRequestPanel
																	rootPath={rootPath}
																	branch={s.branch}
																	onFileSelect={handlePrFileSelect}
																/>
															}
															symbolsContent={
																<SymbolOutlinePanel
																	filePath={s.activeTab?.path ?? null}
																	language={s.activeTab?.language ?? null}
																	rootPath={rootPath}
																	onSelectSymbol={(line) => {
																		if (s.activeTab?.path) {
																			setCenterTab("editor");
																			s.dispatchEditor({
																				type: "SET_PENDING_REVEAL",
																				reveal: {
																					path: s.activeTab.path,
																					line,
																				},
																			});
																		}
																	}}
																/>
															}
														/>
													</div>
												</Panel>
												<Separator />
												<Panel
													id="right-bottom"
													panelRef={rightBottomRef}
													defaultSize="50%"
													minSize="20%"
													collapsible
													collapsedSize={31}
													onResize={(size) =>
														s.setRightBottomCollapsed(size.inPixels <= 31)
													}
												>
													<div
														data-testid="review"
														className="h-full overflow-hidden border-t border-border"
													>
														<RightSidebarBottom
															rootPath={rootPath}
															theme={settings.theme}
															settings={settings}
															threads={s.threads}
															onThreadClick={handleThreadClick}
															onDeleteThread={s.removeThread}
															onResolveThread={s.resolveThread}
															onSendToTerminal={handleThreadsSent}
															showResolvedThreads={s.showResolvedThreads}
															onToggleShowResolved={s.toggleShowResolvedThreads}
															onToggleCollapse={handleToggleRightBottom}
															collapsed={s.rightBottomCollapsed}
															aiTaskThreadIds={s.aiTaskThreadIds}
															onOpenThreadAILog={s.handleOpenThreadAIModal}
															initialActiveTab={
																s.rightBottomActiveTab as RightBottomTab
															}
															onActiveTabChange={s.setRightBottomActiveTab}
														/>
													</div>
												</Panel>
											</Group>
										</div>
									</div>
								</Panel>
							</Group>
						</TabsContent>
					</div>
				</div>

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
					onDismiss={() =>
						s.dispatchGit({ type: "SET_GIT_ERROR", error: null })
					}
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
					repoPaths={[rootPath]}
				/>
				<PostToPrPreview
					open={!!s.pendingPostToPr}
					summary={s.pendingPostToPr?.summary ?? ""}
					loading={s.postToPrLoading}
					onPost={s.handlePostToPrConfirm}
					onCancel={s.handlePostToPrCancel}
				/>
				<ThreadAIModal
					open={s.threadAIModalOpen}
					onOpenChange={s.setThreadAIModalOpen}
					tasks={s.threadAI.taskMap}
					onCancelTask={s.threadAI.cancelTask}
					initialThreadId={s.threadAIInitialThreadId}
				/>
			</EditorContext.Provider>
		</GitStatusProvider>
	);
}

export function MainLayout({
	selectedRootPath,
	settings,
	onSettingsSave,
	leftNav,
}: MainLayoutProps) {
	const leftNavRef = useRef<PanelImperativeHandle>(null);
	const rightPanelRef = useRef<PanelImperativeHandle>(null);

	const [leftNavVisible, setLeftNavVisible] = useState(true);
	const [rightVisible, setRightVisible] = useState(true);
	const [centerTab, setCenterTab] = useState("workflow");
	const switchToEditor = useCallback(() => setCenterTab("editor"), []);

	// --- Workspace state persistence ---
	const { internalStateMapRef, getInitialState } = useWorkspacePersistence({
		selectedRootPath,
		centerTab,
		leftNavVisible,
		rightVisible,
		setCenterTab,
		leftNavRef,
		rightPanelRef,
	});

	const { branch } = useCurrentBranch(selectedRootPath);
	const { baseBranch, setBaseBranch, localBranches } = useBaseBranch(
		selectedRootPath,
		branch,
	);

	const branchSelector = useMemo(
		() => (
			<BranchSelector
				branchName={branch}
				baseBranch={baseBranch}
				localBranches={localBranches}
				onBaseChange={setBaseBranch}
			/>
		),
		[branch, baseBranch, localBranches, setBaseBranch],
	);

	const handleLeftNavResize = useCallback((size: PanelSize) => {
		const visible = size.asPercentage > 0;
		setLeftNavVisible((prev) => (prev === visible ? prev : visible));
	}, []);
	const handleRightResize = useCallback((size: PanelSize) => {
		const visible = size.asPercentage > 0;
		setRightVisible((prev) => (prev === visible ? prev : visible));
	}, []);

	const leftToggle = useMemo<TogglePanel>(
		() => ({
			id: "left-nav",
			icon: PanelLeft,
			label: "Sidebar",
			visible: leftNavVisible,
			onToggle: () => {
				const panel = leftNavRef.current;
				if (!panel) return;
				panel.isCollapsed() ? panel.expand() : panel.collapse();
			},
		}),
		[leftNavVisible],
	);

	const togglePanels = useMemo<TogglePanel[]>(
		() => [
			{
				id: "right",
				icon: PanelRight,
				label: "Right Sidebar",
				visible: rightVisible,
				onToggle: () => {
					setRightVisible((prev) => !prev);
				},
			},
		],
		[rightVisible],
	);

	const rightSlotContent = useMemo(() => branchSelector, [branchSelector]);

	return (
		<div className="h-screen w-screen overflow-hidden bg-background text-foreground">
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
					<div className="flex flex-col h-full overflow-hidden border-r border-border">
						<div
							data-tauri-drag-region
							className="flex items-center justify-end h-[34px] px-1 shrink-0"
						>
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										variant="ghost"
										size="icon"
										className={cn(
											"h-6 w-6",
											leftToggle.visible
												? "text-foreground"
												: "text-muted-foreground",
										)}
										onClick={leftToggle.onToggle}
										aria-label={`Toggle ${leftToggle.label}`}
									>
										<leftToggle.icon className="size-4" />
									</Button>
								</TooltipTrigger>
								<TooltipContent side="bottom">
									{leftToggle.label}
								</TooltipContent>
							</Tooltip>
						</div>
						<div className="flex-1 overflow-hidden">{leftNav}</div>
					</div>
				</Panel>
				<Separator />
				<Panel id="main-area" minSize="30%">
					<Tabs
						value={centerTab}
						onValueChange={setCenterTab}
						className="h-full gap-0"
					>
						{selectedRootPath ? (
							<WorktreeContent
								key={selectedRootPath}
								rootPath={selectedRootPath}
								settings={settings}
								onSettingsSave={onSettingsSave}
								rightPanelRef={rightPanelRef}
								onRightResize={handleRightResize}
								onSwitchToEditor={switchToEditor}
								centerTab={centerTab}
								setCenterTab={setCenterTab}
								leftPanels={leftNavVisible ? undefined : [leftToggle]}
								branchSelector={rightSlotContent}
								togglePanels={togglePanels}
								initialWorkspaceState={getInitialState(selectedRootPath)}
								internalStateMapRef={internalStateMapRef}
							/>
						) : (
							<div className="flex flex-col h-full">
								<ViewToolbar
									leftPanels={leftNavVisible ? undefined : [leftToggle]}
									rightPanels={togglePanels}
									rightSlot={rightSlotContent}
								/>
								<EmptyState
									title="No worktree selected"
									description="Select a worktree from the sidebar to start working"
								/>
							</div>
						)}
					</Tabs>
				</Panel>
			</Group>
		</div>
	);
}
