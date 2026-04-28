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
import { AgentChatPanel } from "@/components/panels/AgentChatPanel";
import { ReviewPanel } from "@/components/panels/ReviewPanel";
import { RightSidebarBottom } from "@/components/panels/RightSidebarBottom";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
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
import type { MentionReference } from "@/types/session";
import type { AppSettings } from "@/types/settings";
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
	leftPanels,
	branchSelector,
	togglePanels,
	initialWorkspaceState,
	internalStateMapRef,
	baseBranch,
}: {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	rightPanelRef: React.Ref<PanelImperativeHandle | null>;
	onRightResize: (size: PanelSize) => void;
	leftPanels?: TogglePanel[];
	branchSelector: React.ReactNode;
	togglePanels: TogglePanel[];
	initialWorkspaceState?: WorkspaceState;
	internalStateMapRef: React.MutableRefObject<
		Map<string, InternalWorktreeState>
	>;
	baseBranch: string | null;
}) {
	const rightBottomRef = useRef<PanelImperativeHandle>(null);
	const reviewRef = useRef<PanelImperativeHandle>(null);
	const [navigateToFile, setNavigateToFile] = useState<{
		path: string;
		line?: number;
	} | null>(null);

	const worktreeName = useMemo(() => {
		const parts = rootPath.split("/");
		return parts[parts.length - 1] ?? "";
	}, [rootPath]);

	const sendAgentMessageRef = useRef<
		((content: string, mentions?: MentionReference[]) => Promise<void>) | null
	>(null);

	const handleSendToAgent = useCallback(
		async (message: string, mentions?: MentionReference[]) => {
			await sendAgentMessageRef.current?.(message, mentions);
		},
		[],
	);

	const handleCommentClick = useCallback(
		(filePath: string, lineNumber?: number) => {
			setNavigateToFile({ path: filePath, line: lineNumber });
		},
		[],
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

	return (
		<>
			{/* Center */}
			<Panel id="center" minSize="30%">
				<div className="h-full relative overflow-hidden flex flex-col">
					<ViewToolbar leftPanels={leftPanels} rightSlot={branchSelector} />
					<div className="flex-1 overflow-hidden">
						<AgentChatPanel
							worktreePath={rootPath}
							registerDropZone={s.registerDropZone}
							sendMessageRef={sendAgentMessageRef}
						/>
					</div>
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
					<RightPanelHeader panels={togglePanels} />
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
										baseBranch={baseBranch}
										defaultDiffBase={settings.defaultDiffBase}
										defaultDiffMode={settings.defaultDiffMode}
										diffOnlyMode={s.diffOnlyMode}
										onDiffOnlyModeChange={s.setDiffOnlyMode}
										navigateToFile={navigateToFile}
										onSendToAgent={handleSendToAgent}
										initialSelectedFile={s.selectedDiffFile}
										onSelectedFileChange={s.setSelectedDiffFile}
									/>
								</div>
							</Panel>
							<Separator />
							<Panel
								id="right-bottom"
								panelRef={rightBottomRef}
								defaultSize="40%"
								minSize="20%"
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
										onSendToAgent={handleSendToAgent}
										onCommentClick={handleCommentClick}
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
		</>
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
				{branchSelector}
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
		[branchSelector, rightVisible, togglePanels],
	);

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
								branchSelector={rightSlotContent}
								togglePanels={togglePanels}
								initialWorkspaceState={getInitialState(selectedRootPath)}
								internalStateMapRef={internalStateMapRef}
								baseBranch={baseBranch}
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
