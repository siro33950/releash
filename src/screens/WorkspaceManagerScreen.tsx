import { type IJsonModel, Layout, Model, type TabNode } from "flexlayout-react";
import { FolderOpen, Globe, Loader2, Settings } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import {
	ActivityBar,
	type ActivityBarItem,
} from "@/components/layout/ActivityBar";
import { RemotePanel } from "@/components/panels/RemotePanel";
import { SettingsPanel } from "@/components/panels/SettingsPanel";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Button } from "@/components/ui/button";
import { RepoKanbanBoard } from "@/components/workspace/RepoKanbanBoard";
import type { ProviderStatus } from "@/types/git";
import type { AppSettings } from "@/types/settings";

const initialJson: IJsonModel = {
	global: {
		tabEnableClose: false,
		tabEnableDrag: false,
		tabEnableRename: false,
		tabSetEnableMaximize: false,
		tabSetEnableDeleteWhenEmpty: false,
		splitterSize: 1,
		splitterExtra: 4,
	},
	layout: {
		type: "row",
		weight: 100,
		children: [
			{
				type: "tabset",
				id: "sidebar",
				weight: 15,
				enableDrag: false,
				enableDrop: false,
				enableTabStrip: false,
				minWidth: 160,
				children: [
					{
						type: "tab",
						id: "sidebar-content",
						component: "sidebar",
						enableClose: false,
						enableDrag: false,
					},
				],
			},
			{
				type: "tabset",
				id: "kanban",
				weight: 55,
				enableDrag: false,
				enableDrop: false,
				enableTabStrip: false,
				children: [
					{
						type: "tab",
						id: "kanban-content",
						component: "kanban",
						enableClose: false,
						enableDrag: false,
					},
				],
			},
			{
				type: "tabset",
				id: "terminal",
				weight: 30,
				enableDrag: false,
				enableDrop: false,
				enableTabStrip: false,
				minWidth: 200,
				children: [
					{
						type: "tab",
						id: "terminal-content",
						component: "terminal",
						enableClose: false,
						enableDrag: false,
					},
				],
			},
		],
	},
};

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
	const [activeView, setActiveView] = useState<string>("remote");

	useEffect(() => {
		if (requestedView) {
			setActiveView(requestedView);
			onRequestedViewHandled?.();
		}
	}, [requestedView, onRequestedViewHandled]);

	const activityBarItems: ActivityBarItem[] = useMemo(
		() => [
			{
				id: "remote",
				icon: <Globe className="size-5" />,
				title: "Remote",
			},
		],
		[],
	);

	const activityBarBottomItems: ActivityBarItem[] = useMemo(
		() => [
			{
				id: "settings",
				icon: <Settings className="size-5" />,
				title: "Settings",
			},
		],
		[],
	);

	const model = useMemo(() => Model.fromJson(initialJson), []);

	const factory = useCallback(
		(node: TabNode): ReactNode => {
			switch (node.getComponent()) {
				case "sidebar":
					if (activeView === "remote") {
						return <RemotePanel rootPaths={repoPaths} />;
					}
					if (activeView === "settings") {
						return (
							<SettingsPanel settings={settings} onSave={onSettingsSave} />
						);
					}
					return null;
				case "kanban":
					return (
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
					);
				case "terminal":
					return <TerminalPanel theme={settings.theme} sessionKey="kanban" />;
				default:
					return null;
			}
		},
		[
			activeView,
			repoPaths,
			settings,
			providerStatuses,
			initializing,
			onSettingsSave,
			onSelectWorktree,
			onAddRepo,
			onRemoveRepo,
		],
	);

	if (repoPaths.length === 0 && initializing) {
		return (
			<div className="flex flex-col items-center justify-center h-full w-full bg-background text-foreground gap-4">
				<Loader2 className="size-6 text-muted-foreground animate-spin" />
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full w-full bg-background text-foreground">
			<div className="flex flex-1 min-h-0">
				<ActivityBar
					items={activityBarItems}
					bottomItems={activityBarBottomItems}
					activeItem={activeView}
					onItemClick={setActiveView}
				/>
				<div className="flex-1 relative overflow-hidden">
					<Layout model={model} factory={factory} />
				</div>
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
