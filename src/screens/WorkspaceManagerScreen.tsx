import { FolderOpen, Globe, Loader2, Settings } from "lucide-react";
import { useMemo, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
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

interface WorkspaceManagerScreenProps {
	repoPaths: string[];
	settings: AppSettings;
	providerStatuses: Record<string, ProviderStatus | null>;
	initializing?: boolean;
	isActive?: boolean;
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
	onSettingsSave,
	onSelectWorktree,
	onAddRepo,
	onRemoveRepo,
}: WorkspaceManagerScreenProps) {
	const [activeView, setActiveView] = useState<string>("remote");

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

	if (repoPaths.length === 0 && initializing) {
		return (
			<div className="flex flex-col items-center justify-center h-full w-full bg-background text-foreground gap-4">
				<Loader2 className="size-6 text-muted-foreground animate-spin" />
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full w-full bg-background text-foreground">
			{/* Main content: ActivityBar + Sidebar + Kanban + Terminal */}
			<div className="flex flex-1 min-h-0">
				{/* ActivityBar */}
				<ActivityBar
					items={activityBarItems}
					bottomItems={activityBarBottomItems}
					activeItem={activeView}
					onItemClick={setActiveView}
				/>

				<Group orientation="horizontal" className="flex-1">
					{/* Sidebar */}
					<Panel
						id="sidebar"
						defaultSize="15"
						minSize={10}
						maxSize="30"
						collapsible={false}
					>
						{activeView === "remote" && <RemotePanel rootPaths={repoPaths} />}
						{activeView === "settings" && (
							<SettingsPanel settings={settings} onSave={onSettingsSave} />
						)}
					</Panel>

					<Separator className="w-px bg-border hover:bg-primary/50 cursor-col-resize" />

					{/* Kanban area: vertically scrollable, each repo's kanban */}
					<Panel id="kanban" defaultSize="55" minSize={20} collapsible={false}>
						<div className="h-full flex flex-col">
							{/* Kanban header */}
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

					<Separator className="w-px bg-border hover:bg-primary/50 cursor-col-resize" />

					{/* Terminal (home directory, no agent startup) */}
					<Panel
						id="terminal"
						defaultSize="30"
						minSize={10}
						maxSize="60"
						collapsible={false}
					>
						<TerminalPanel theme={settings.theme} sessionKey="kanban" />
					</Panel>
				</Group>
			</div>

			{/* Status bar */}
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
