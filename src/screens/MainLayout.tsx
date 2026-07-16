import { invoke } from "@tauri-apps/api/core";
import { PanelLeft, PanelRight } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	Group,
	Panel,
	type PanelImperativeHandle,
	type PanelSize,
	Separator,
} from "react-resizable-panels";
import { BranchSelector } from "@/components/layout/BranchSelector";

import { RightPanelHeader } from "@/components/layout/RightPanelHeader";
import { type TogglePanel, ViewToolbar } from "@/components/layout/ViewToolbar";
import { NodeContentView } from "@/components/panels/NodeContentView";
import { ReviewPanel } from "@/components/panels/ReviewPanel";
import { RightSidebarBottom } from "@/components/panels/RightSidebarBottom";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import {
	AgentChatProvider,
	useAgentChatContext,
} from "@/contexts/AgentChatContext";
import { ReviewThreadHandoffProvider } from "@/contexts/ReviewThreadHandoffContext";
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
	GitErrorDialog,
} from "@/screens/WorktreeViewDialogs";
import type { ThreadNavigationTarget } from "@/types/diffComment";
import type { AgentEditorSelection, MentionReference } from "@/types/session";
import type { AppSettings } from "@/types/settings";
import type { WorkspaceState } from "@/types/workspace-state";
import type {
	CenterSelection,
	NewSessionCreationRequest,
} from "@/types/workspace-tree";

interface MainLayoutProps {
	selectedRootPath: string | null;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	leftNav: React.ReactNode;
	centerSelection?: CenterSelection | null;
	newSessionCreationRequest?: NewSessionCreationRequest | null;
	onCenterSelectionResolved?: (selection: CenterSelection) => void;
}

function NewSessionCreationBridge({
	worktreePath,
	request,
	onCreated,
}: {
	worktreePath: string;
	request: NewSessionCreationRequest | null;
	onCreated?: (selection: CenterSelection) => void;
}) {
	const { createNewSession } = useAgentChatContext();
	const requestTaskRef = useRef<{
		requestId: number;
		task: Promise<string | null>;
	} | null>(null);
	const deliveredRequestIdRef = useRef<number | null>(null);

	useEffect(() => {
		if (!request || request.worktreePath !== worktreePath) return;
		if (deliveredRequestIdRef.current === request.requestId) return;
		let cancelled = false;

		let task =
			requestTaskRef.current?.requestId === request.requestId
				? requestTaskRef.current.task
				: null;
		if (task == null) {
			task = createNewSession().then(async (sessionId) => {
				if (!sessionId) return null;
				return invoke<string | null>("get_workspace_session_node_id", {
					worktreePath,
					sessionId,
				});
			});
			requestTaskRef.current = { requestId: request.requestId, task };
		}

		void task
			.then((nodeId) => {
				if (!nodeId || cancelled) return;
				if (requestTaskRef.current?.requestId === request.requestId) {
					requestTaskRef.current = null;
				}
				deliveredRequestIdRef.current = request.requestId;
				window.dispatchEvent(
					new CustomEvent("workspace-tree-refresh", {
						detail: { worktreePath },
					}),
				);
				onCreated?.({ kind: "node", worktreePath, nodeId });
			})
			.catch((error: unknown) => {
				if (!cancelled) {
					console.warn("[NewSessionCreationBridge] create failed", error);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [createNewSession, onCreated, request, worktreePath]);

	return null;
}

function WorktreeContent({
	rootPath,
	settings,
	onSettingsSave,
	rightPanelRef,
	onRightResize,
	leftPanels,
	branchSelector,
	rightSlot,
	togglePanels,
	centerSelection,
	newSessionCreationRequest,
	onCenterSelectionResolved,
	initialWorkspaceState,
	internalStateMapRef,
}: {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	rightPanelRef: React.RefObject<PanelImperativeHandle | null>;
	onRightResize: (size: PanelSize) => void;
	leftPanels?: TogglePanel[];
	branchSelector: React.ReactNode;
	rightSlot?: React.ReactNode;
	togglePanels: TogglePanel[];
	centerSelection: CenterSelection | null;
	newSessionCreationRequest?: NewSessionCreationRequest | null;
	onCenterSelectionResolved?: (selection: CenterSelection) => void;
	initialWorkspaceState?: WorkspaceState;
	internalStateMapRef: React.MutableRefObject<
		Map<string, InternalWorktreeState>
	>;
}) {
	const rightBottomRef = useRef<PanelImperativeHandle>(null);
	const reviewRef = useRef<PanelImperativeHandle>(null);
	const [navigateToThread, setNavigateToThread] =
		useState<ThreadNavigationTarget | null>(null);
	const [activeEditorSelection, setActiveEditorSelection] =
		useState<AgentEditorSelection | null>(null);

	const worktreeName = rootPath;

	const sendAgentMessageRef = useRef<
		((content: string, mentions?: MentionReference[]) => Promise<void>) | null
	>(null);

	const handleSendToAgent = useCallback(
		async (message: string, mentions?: MentionReference[]) => {
			await sendAgentMessageRef.current?.(message, mentions);
		},
		[],
	);

	const handleThreadClick = useCallback((target: ThreadNavigationTarget) => {
		setNavigateToThread(target);
	}, []);

	const handleLineRangeSelected = useCallback(
		(filePath: string, startLine: number, endLine: number) => {
			if (!filePath || startLine < 1 || endLine < 1) {
				setActiveEditorSelection(null);
				return;
			}
			const absolutePath = filePath.startsWith("/")
				? filePath
				: `${rootPath}/${filePath}`;
			setActiveEditorSelection({
				filePath: absolutePath,
				startLine: Math.min(startLine, endLine),
				endLine: Math.max(startLine, endLine),
			});
		},
		[rootPath],
	);

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

	// Restore review collapsed state from initialWorkspaceState
	const initialReviewCollapsed =
		initialWorkspaceState?.layout.reviewCollapsed ?? false;
	useEffect(() => {
		if (!initialReviewCollapsed) return;
		const panel = reviewRef.current;
		if (!panel) return;
		requestAnimationFrame(() => {
			panel.collapse();
		});
	}, [initialReviewCollapsed]);

	const s = useWorktreeState({
		rootPath,
		settings,
		onSettingsSave,
		isActive: true,
		initialWorkspaceState,
		internalStateMapRef,
	});
	const editorState = internalStateMapRef.current.get(rootPath);
	const openEditorPaths = (
		editorState?.tabs ??
		initialWorkspaceState?.tabs.editors ??
		[]
	).map((tab) => tab.path);
	const activeEditorPath =
		editorState?.activeEditorPath ??
		initialWorkspaceState?.tabs.activeEditorPath ??
		null;
	const scopedCenterSelection =
		centerSelection?.worktreePath === rootPath ? centerSelection : null;
	const handleOpenDiffFile = useCallback(
		(filePath: string) => {
			s.setSelectedDiffFile(filePath);
			if (rightPanelRef.current?.isCollapsed()) {
				rightPanelRef.current.expand();
			}
			if (reviewRef.current?.isCollapsed()) {
				reviewRef.current.expand();
			}
		},
		[rightPanelRef, s.setSelectedDiffFile],
	);

	return (
		<AgentChatProvider worktreePath={rootPath}>
			<NewSessionCreationBridge
				worktreePath={rootPath}
				request={newSessionCreationRequest ?? null}
				onCreated={onCenterSelectionResolved}
			/>
			<ReviewThreadHandoffProvider worktreeName={worktreeName}>
				{/* Center */}
				<Panel id="center" defaultSize="50%" minSize="30%">
					<div className="h-full relative overflow-hidden flex flex-col">
						<NodeContentView
							worktreePath={rootPath}
							nodeId={scopedCenterSelection?.nodeId ?? null}
							leftPanels={leftPanels}
							rightSlot={rightSlot}
							activeEditorPath={activeEditorPath}
							openEditorPaths={openEditorPaths}
							activeEditorSelection={activeEditorSelection}
							registerDropZone={s.registerDropZone}
							sendMessageRef={sendAgentMessageRef}
							onOpenDiffFile={handleOpenDiffFile}
						/>
					</div>
				</Panel>
				<Separator />
				{/* Right Sidebar */}
				<Panel
					id="right"
					panelRef={rightPanelRef}
					defaultSize="50%"
					minSize={280}
					collapsible
					collapsedSize="0%"
					onResize={onRightResize}
				>
					<div className="flex flex-col h-full border-l border-border">
						<RightPanelHeader panels={togglePanels} leftSlot={branchSelector} />
						<div className="flex-1 overflow-hidden">
							<Group orientation="vertical">
								<Panel
									id="review"
									panelRef={reviewRef}
									defaultSize="60%"
									minSize="20%"
									collapsible
									collapsedSize="0%"
									onResize={(size) =>
										s.setReviewCollapsed(size.asPercentage <= 0)
									}
								>
									<div className="h-full overflow-hidden">
										<ReviewPanel
											rootPath={rootPath}
											defaultDiffBase={settings.defaultDiffBase}
											defaultDiffMode={settings.defaultDiffMode}
											diffOnlyMode={s.diffOnlyMode}
											onDiffOnlyModeChange={s.setDiffOnlyMode}
											navigateToThread={navigateToThread}
											onSendToAgent={handleSendToAgent}
											initialSelectedFile={s.selectedDiffFile}
											onSelectedFileChange={s.setSelectedDiffFile}
											onLineRangeSelected={handleLineRangeSelected}
										/>
									</div>
								</Panel>
								<Separator />
								<Panel
									id="right-bottom"
									panelRef={rightBottomRef}
									defaultSize={300}
									minSize="20%"
									groupResizeBehavior="preserve-pixel-size"
									collapsible
									collapsedSize={31}
									onResize={(size) =>
										s.setRightBottomCollapsed(size.inPixels <= 31)
									}
								>
									<div
										data-testid="right-bottom-content"
										className="h-full overflow-hidden"
									>
										<RightSidebarBottom
											rootPath={rootPath}
											theme={settings.theme}
											worktreeName={worktreeName}
											onThreadClick={handleThreadClick}
											onToggleCollapse={handleToggleRightBottom}
											collapsed={s.rightBottomCollapsed}
										/>
									</div>
								</Panel>
							</Group>
						</div>
					</div>
				</Panel>

				{/* Dialogs */}
				<GitErrorDialog
					error={s.gitError}
					onOpenChange={(o) => {
						if (!o) s.dispatchGit({ type: "SET_GIT_ERROR", error: null });
					}}
					onDismiss={() =>
						s.dispatchGit({ type: "SET_GIT_ERROR", error: null })
					}
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
			</ReviewThreadHandoffProvider>
		</AgentChatProvider>
	);
}

export function MainLayout({
	selectedRootPath,
	settings,
	onSettingsSave,
	leftNav,
	centerSelection,
	newSessionCreationRequest,
	onCenterSelectionResolved,
}: MainLayoutProps) {
	const leftNavRef = useRef<PanelImperativeHandle>(null);
	const rightPanelRef = useRef<PanelImperativeHandle>(null);

	const [leftNavVisible, setLeftNavVisible] = useState(true);
	const [rightVisible, setRightVisible] = useState(true);
	// --- Workspace state persistence ---
	const { internalStateMapRef, getInitialState, stateReady } =
		useWorkspacePersistence({
			selectedRootPath,
			centerTab: "agent",
			leftNavVisible,
			rightVisible,
			setCenterTab: () => {},
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
					const panel = rightPanelRef.current;
					if (!panel) return;
					panel.isCollapsed() ? panel.expand() : panel.collapse();
				},
			},
		],
		[rightVisible],
	);

	const rightSlotContent = useMemo(
		() => (
			<>
				{!rightVisible &&
					togglePanels.map((p) => (
						<Tooltip key={p.id}>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className={cn(
										"h-6 w-6",
										p.visible ? "text-foreground" : "text-muted-foreground",
									)}
									onClick={p.onToggle}
									aria-label={`Toggle ${p.label}`}
								>
									<p.icon className="size-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent side="bottom">{p.label}</TooltipContent>
						</Tooltip>
					))}
			</>
		),
		[rightVisible, togglePanels],
	);

	return (
		<div className="h-screen w-screen overflow-hidden bg-background text-foreground">
			<Group orientation="horizontal" className="h-full">
				<Panel
					id="left-nav"
					panelRef={leftNavRef}
					defaultSize={230}
					minSize={230}
					groupResizeBehavior="preserve-pixel-size"
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
					<Group orientation="horizontal" className="h-full">
						{selectedRootPath && stateReady ? (
							<WorktreeContent
								key={selectedRootPath}
								rootPath={selectedRootPath}
								settings={settings}
								onSettingsSave={onSettingsSave}
								rightPanelRef={rightPanelRef}
								onRightResize={handleRightResize}
								leftPanels={leftNavVisible ? undefined : [leftToggle]}
								branchSelector={branchSelector}
								rightSlot={rightSlotContent}
								togglePanels={togglePanels}
								centerSelection={centerSelection ?? null}
								newSessionCreationRequest={newSessionCreationRequest}
								onCenterSelectionResolved={onCenterSelectionResolved}
								initialWorkspaceState={getInitialState(selectedRootPath)}
								internalStateMapRef={internalStateMapRef}
							/>
						) : (
							<Panel id="center" minSize="30%">
								<div className="flex flex-col h-full">
									<ViewToolbar
										leftPanels={leftNavVisible ? undefined : [leftToggle]}
										rightSlot={rightSlotContent}
									/>
									{!selectedRootPath ? (
										<div className="flex-1 flex items-center justify-center text-muted-foreground text-sm">
											Select a worktree from the sidebar to start working
										</div>
									) : (
										<div className="flex-1" />
									)}
								</div>
							</Panel>
						)}
					</Group>
				</Panel>
			</Group>
		</div>
	);
}
