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
import type { AppSettings } from "@/types/settings";
import type { WorkspaceState } from "@/types/workspace-state";
import type { CenterSelection } from "@/types/workspace-tree";

interface MainLayoutProps {
	selectedRootPath: string | null;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	leftNav: React.ReactNode;
	topBanner?: React.ReactNode;
	centerSelectionByWorktree?: Record<string, CenterSelection | null>;
	onCenterNodeMissing?: (worktreePath: string, nodeId: string) => void;
	onCenterSessionAttachmentConsumed?: (
		worktreePath: string,
		nodeId: string,
		agentSessionId: string,
	) => void;
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
	onCenterNodeMissing,
	onCenterSessionAttachmentConsumed,
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
	onCenterNodeMissing?: (worktreePath: string, nodeId: string) => void;
	onCenterSessionAttachmentConsumed?: (
		worktreePath: string,
		nodeId: string,
		agentSessionId: string,
	) => void;
	initialWorkspaceState?: WorkspaceState;
	internalStateMapRef: React.MutableRefObject<
		Map<string, InternalWorktreeState>
	>;
}) {
	const rightBottomRef = useRef<PanelImperativeHandle>(null);
	const reviewRef = useRef<PanelImperativeHandle>(null);
	const [navigateToThread, setNavigateToThread] =
		useState<ThreadNavigationTarget | null>(null);

	const worktreeName = rootPath;

	const handleThreadClick = useCallback((target: ThreadNavigationTarget) => {
		setNavigateToThread(target);
	}, []);

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
	const scopedCenterSelection =
		centerSelection?.worktreePath === rootPath ? centerSelection : null;

	return (
		<ReviewThreadHandoffProvider worktreeName={worktreeName}>
			{/* Center */}
			<Panel id="center" defaultSize="50%" minSize="30%">
				<div className="h-full relative overflow-hidden flex flex-col">
					{scopedCenterSelection?.kind === "agent_session_launching" ? (
						<div
							className="flex h-full flex-col items-center justify-center gap-3 bg-background p-4 text-sm"
							data-testid="agent-session-launching"
						>
							{scopedCenterSelection.error ? (
								<div role="alert" className="text-destructive">
									{scopedCenterSelection.error}
								</div>
							) : (
								<div>Opening AgentSession...</div>
							)}
						</div>
					) : (
						<NodeContentView
							worktreePath={rootPath}
							theme={settings.theme}
							nodeId={scopedCenterSelection?.nodeId ?? null}
							leftPanels={leftPanels}
							rightSlot={rightSlot}
							onNodeMissing={onCenterNodeMissing}
							initialSessionAttachment={
								scopedCenterSelection?.initialSessionAttachment
							}
							onInitialSessionConsumed={(agentSessionId) => {
								if (!scopedCenterSelection) return;
								onCenterSessionAttachmentConsumed?.(
									rootPath,
									scopedCenterSelection.nodeId,
									agentSessionId,
								);
							}}
						/>
					)}
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
										initialSelectedFile={s.selectedDiffFile}
										onSelectedFileChange={s.setSelectedDiffFile}
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
				onDismiss={() => s.dispatchGit({ type: "SET_GIT_ERROR", error: null })}
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
	);
}

function createRightTogglePanel(
	panelRef: React.RefObject<PanelImperativeHandle | null>,
	visible: boolean,
): TogglePanel {
	return {
		id: "right",
		icon: PanelRight,
		label: "Right Sidebar",
		visible,
		onToggle: () => {
			const panel = panelRef.current;
			if (!panel) return;
			panel.isCollapsed() ? panel.expand() : panel.collapse();
		},
	};
}

interface WorktreePaneProps {
	rootPath: string;
	active: boolean;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	activeRightPanelRef: React.MutableRefObject<PanelImperativeHandle | null>;
	onRightVisibleChange: (rootPath: string, visible: boolean) => void;
	leftPanels?: TogglePanel[];
	rightSlot?: React.ReactNode;
	centerSelection: CenterSelection | null;
	onCenterNodeMissing?: (worktreePath: string, nodeId: string) => void;
	onCenterSessionAttachmentConsumed?: (
		worktreePath: string,
		nodeId: string,
		agentSessionId: string,
	) => void;
	initialWorkspaceState?: WorkspaceState;
	internalStateMapRef: React.MutableRefObject<
		Map<string, InternalWorktreeState>
	>;
}

/// keep-mounted pane。非activeでもmountを維持し、visibility切替だけで復帰する。
/// display:noneではなくvisibilityを使い、panel/xtermの実サイズを保持する。
function WorktreePane({
	rootPath,
	active,
	settings,
	onSettingsSave,
	activeRightPanelRef,
	onRightVisibleChange,
	leftPanels,
	rightSlot,
	centerSelection,
	onCenterNodeMissing,
	onCenterSessionAttachmentConsumed,
	initialWorkspaceState,
	internalStateMapRef,
}: WorktreePaneProps) {
	const ownRightPanelRef = useRef<PanelImperativeHandle>(null);
	const [rightVisible, setRightVisible] = useState(true);
	useEffect(() => {
		if (!active) return;
		activeRightPanelRef.current = ownRightPanelRef.current;
		return () => {
			if (activeRightPanelRef.current === ownRightPanelRef.current) {
				activeRightPanelRef.current = null;
			}
		};
	}, [active, activeRightPanelRef]);
	const handleRightResize = useCallback(
		(size: PanelSize) => {
			const visible = size.asPercentage > 0;
			setRightVisible((prev) => (prev === visible ? prev : visible));
			onRightVisibleChange(rootPath, visible);
		},
		[onRightVisibleChange, rootPath],
	);

	const { branch } = useCurrentBranch(rootPath);
	const { baseBranch, setBaseBranch, localBranches } = useBaseBranch(
		rootPath,
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
	const togglePanels = useMemo<TogglePanel[]>(
		() => [createRightTogglePanel(ownRightPanelRef, rightVisible)],
		[rightVisible],
	);

	return (
		<div
			data-testid={`worktree-pane-${rootPath}`}
			data-active={active}
			className={cn(
				"absolute inset-0",
				active ? "visible" : "invisible pointer-events-none",
			)}
			aria-hidden={!active}
		>
			<Group orientation="horizontal" className="h-full">
				<WorktreeContent
					rootPath={rootPath}
					settings={settings}
					onSettingsSave={onSettingsSave}
					rightPanelRef={ownRightPanelRef}
					onRightResize={handleRightResize}
					leftPanels={leftPanels}
					branchSelector={branchSelector}
					rightSlot={rightSlot}
					togglePanels={togglePanels}
					centerSelection={centerSelection}
					onCenterNodeMissing={onCenterNodeMissing}
					onCenterSessionAttachmentConsumed={onCenterSessionAttachmentConsumed}
					initialWorkspaceState={initialWorkspaceState}
					internalStateMapRef={internalStateMapRef}
				/>
			</Group>
		</div>
	);
}

export function MainLayout({
	selectedRootPath,
	settings,
	onSettingsSave,
	leftNav,
	topBanner,
	centerSelectionByWorktree,
	onCenterNodeMissing,
	onCenterSessionAttachmentConsumed,
}: MainLayoutProps) {
	const leftNavRef = useRef<PanelImperativeHandle>(null);
	const rightPanelRef = useRef<PanelImperativeHandle>(null);
	// keep-mounted panes: 一度開いたworktreeのpaneはmountしたまま表示切替する。
	// 復帰時のremount（Review再取得・terminal再attach）を排除する。
	const MAX_MOUNTED_PANES = 5;
	const [mountedRootPaths, setMountedRootPaths] = useState<string[]>([]);
	useEffect(() => {
		if (!selectedRootPath) return;
		setMountedRootPaths((current) => {
			const next = [
				selectedRootPath,
				...current.filter((path) => path !== selectedRootPath),
			];
			return next.slice(0, MAX_MOUNTED_PANES);
		});
	}, [selectedRootPath]);

	const [leftNavVisible, setLeftNavVisible] = useState(true);
	const [rightVisible, setRightVisible] = useState(true);
	const rightVisibleByPaneRef = useRef(new Map<string, boolean>());
	const selectedRootPathRef = useRef(selectedRootPath);
	selectedRootPathRef.current = selectedRootPath;
	const handlePaneRightVisibleChange = useCallback(
		(rootPath: string, visible: boolean) => {
			rightVisibleByPaneRef.current.set(rootPath, visible);
			if (rootPath === selectedRootPathRef.current) {
				setRightVisible((prev) => (prev === visible ? prev : visible));
			}
		},
		[],
	);
	useEffect(() => {
		if (!selectedRootPath) return;
		const visible = rightVisibleByPaneRef.current.get(selectedRootPath) ?? true;
		setRightVisible((prev) => (prev === visible ? prev : visible));
	}, [selectedRootPath]);
	useEffect(() => {
		const mounted = new Set(mountedRootPaths);
		for (const path of [...rightVisibleByPaneRef.current.keys()]) {
			if (!mounted.has(path)) {
				rightVisibleByPaneRef.current.delete(path);
			}
		}
	}, [mountedRootPaths]);
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

	const handleLeftNavResize = useCallback((size: PanelSize) => {
		const visible = size.asPercentage > 0;
		setLeftNavVisible((prev) => (prev === visible ? prev : visible));
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
		() => [createRightTogglePanel(rightPanelRef, rightVisible)],
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
					<div className="flex h-full min-h-0 flex-col">
						{topBanner && (
							<div data-testid="main-layout-banner-region" className="shrink-0">
								{topBanner}
							</div>
						)}
						<div
							data-testid="main-layout-content-region"
							className="relative min-h-0 flex-1"
						>
							{stateReady &&
								mountedRootPaths.map((rootPath) => (
									<WorktreePane
										key={rootPath}
										rootPath={rootPath}
										active={rootPath === selectedRootPath}
										settings={settings}
										onSettingsSave={onSettingsSave}
										activeRightPanelRef={rightPanelRef}
										onRightVisibleChange={handlePaneRightVisibleChange}
										leftPanels={leftNavVisible ? undefined : [leftToggle]}
										rightSlot={rightSlotContent}
										centerSelection={
											centerSelectionByWorktree?.[rootPath] ?? null
										}
										onCenterNodeMissing={onCenterNodeMissing}
										onCenterSessionAttachmentConsumed={
											onCenterSessionAttachmentConsumed
										}
										initialWorkspaceState={getInitialState(rootPath)}
										internalStateMapRef={internalStateMapRef}
									/>
								))}
							{(!selectedRootPath || !stateReady) && (
								<div className="absolute inset-0">
									<Group orientation="horizontal" className="h-full">
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
									</Group>
								</div>
							)}
						</div>
					</div>
				</Panel>
			</Group>
		</div>
	);
}
