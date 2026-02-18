import {
	CircleDot,
	FolderOpen,
	Globe,
	Loader2,
	PanelLeft,
	PanelRight,
	Settings,
	StickyNote,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	Group,
	Panel,
	type PanelImperativeHandle,
	type PanelSize,
	Separator,
} from "react-resizable-panels";
import {
	ActivityBar,
	type ActivityBarItem,
} from "@/components/layout/ActivityBar";
import { type TogglePanel, ViewToolbar } from "@/components/layout/ViewToolbar";
import { IssuePanel } from "@/components/panels/IssuePanel";
import { NotionPanel } from "@/components/panels/NotionPanel";
import { RemotePanel } from "@/components/panels/RemotePanel";
import { SettingsPanel } from "@/components/panels/SettingsPanel";
import type { TerminalPanelHandle } from "@/components/panels/TerminalPanel";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Button } from "@/components/ui/button";
import { RepoKanbanBoard } from "@/components/workspace/RepoKanbanBoard";
import type { ProviderStatus } from "@/types/git";
import { type AppSettings, buildTerminalCommand } from "@/types/settings";

interface WorkspaceManagerScreenProps {
	repoPaths: string[];
	settings: AppSettings;
	providerStatuses: Record<string, ProviderStatus | null>;
	initializing?: boolean;
	isActive?: boolean;
	requestedView?: string | null;
	onRequestedViewHandled?: () => void;
	onSettingsSave: (settings: AppSettings) => void;
	onSelectWorktree: (
		path: string,
		branchName?: string,
		repoName?: string,
	) => void;
	onAddRepo: () => void;
	onRemoveRepo: (repoPath: string) => void;
}

export function WorkspaceManagerScreen({
	repoPaths,
	settings,
	providerStatuses,
	initializing = false,
	requestedView,
	onRequestedViewHandled,
	onSettingsSave,
	onSelectWorktree,
	onAddRepo,
	onRemoveRepo,
}: WorkspaceManagerScreenProps) {
	const [activeView, setActiveView] = useState<string>("issues");

	useEffect(() => {
		if (requestedView) {
			setActiveView(requestedView);
			onRequestedViewHandled?.();
		}
	}, [requestedView, onRequestedViewHandled]);

	const activityBarItems: ActivityBarItem[] = useMemo(
		() => [
			{
				id: "issues",
				icon: <CircleDot className="size-5" />,
				title: "Issues",
			},
			{
				id: "notion",
				icon: <StickyNote className="size-5" />,
				title: "Notion Tasks",
			},
		],
		[],
	);

	const activityBarBottomItems: ActivityBarItem[] = useMemo(
		() => [
			{
				id: "remote",
				icon: <Globe className="size-5" />,
				title: "Remote",
			},
			{
				id: "settings",
				icon: <Settings className="size-5" />,
				title: "Settings",
			},
		],
		[],
	);

	const sidebarPanelRef = useRef<PanelImperativeHandle>(null);
	const terminalPanelRef = useRef<PanelImperativeHandle>(null);
	const terminalRef = useRef<TerminalPanelHandle>(null);

	const [sidebarVisible, setSidebarVisible] = useState(true);
	const [terminalVisible, setTerminalVisible] = useState(true);

	const handleSidebarResize = useCallback((size: PanelSize) => {
		const visible = size.asPercentage > 0;
		setSidebarVisible((prev) => (prev === visible ? prev : visible));
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
				id: "terminal",
				icon: PanelRight,
				label: "Terminal",
				visible: terminalVisible,
				onToggle: toggleTerminal,
			},
		],
		[sidebarVisible, terminalVisible, toggleSidebar, toggleTerminal],
	);

	const sidebarContent = useMemo(() => {
		if (activeView === "issues") {
			return (
				<IssuePanel
					repoPaths={repoPaths}
					providerStatuses={providerStatuses}
					onSelectWorktree={onSelectWorktree}
				/>
			);
		}
		if (activeView === "notion") {
			return (
				<NotionPanel
					repoPaths={repoPaths}
					onSelectWorktree={onSelectWorktree}
				/>
			);
		}
		if (activeView === "remote") {
			return (
				<RemotePanel
					rootPaths={repoPaths}
					terminalStartupCommand={buildTerminalCommand(settings)}
				/>
			);
		}
		if (activeView === "settings") {
			return <SettingsPanel settings={settings} onSave={onSettingsSave} />;
		}
		return null;
	}, [
		activeView,
		repoPaths,
		providerStatuses,
		onSelectWorktree,
		settings,
		onSettingsSave,
	]);

	if (repoPaths.length === 0 && initializing) {
		return (
			<div className="flex flex-col items-center justify-center h-full w-full bg-background text-foreground gap-4">
				<Loader2 className="size-6 text-muted-foreground animate-spin" />
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full w-full bg-background text-foreground">
			<ViewToolbar panels={togglePanels} />
			<div className="flex flex-1 min-h-0">
				<ActivityBar
					items={activityBarItems}
					bottomItems={activityBarBottomItems}
					activeItem={activeView}
					onItemClick={setActiveView}
				/>
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
					<Panel id="kanban" minSize="20%">
						<div className="h-full flex flex-col">
							<div className="flex items-center justify-between h-[30px] px-3 border-b border-border shrink-0">
								<span className="text-xs font-semibold uppercase tracking-wide truncate">
									Repositories
								</span>
								<Button
									size="sm"
									variant="ghost"
									className="h-6 px-2 text-xs"
									onClick={onAddRepo}
								>
									<FolderOpen className="size-3.5 mr-1" />
									Open
								</Button>
							</div>
							<div className="flex-1 overflow-y-auto">
								{repoPaths.map((repoPath) => (
									<RepoKanbanBoard
										key={repoPath}
										repoPath={repoPath}
										providerStatus={providerStatuses[repoPath] ?? null}
										onSelectWorktree={onSelectWorktree}
										onRemove={() => onRemoveRepo(repoPath)}
									/>
								))}
								{repoPaths.length === 0 && !initializing && (
									<div className="flex flex-col items-center justify-center h-full gap-4">
										<Button onClick={onAddRepo}>
											<FolderOpen className="size-4 mr-2" />
											Open Folder
										</Button>
									</div>
								)}
							</div>
						</div>
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
								theme={settings.theme}
								sessionKey="kanban"
							/>
						</div>
					</Panel>
				</Group>
			</div>

			<div className="flex items-center h-6 px-3 bg-primary text-primary-foreground text-xs shrink-0">
				<span className="truncate">
					{repoPaths.length > 0
						? `${repoPaths.length} repositor${repoPaths.length === 1 ? "y" : "ies"}`
						: "No repository"}
				</span>
			</div>
		</div>
	);
}
