import { GitBranch, LayoutGrid } from "lucide-react";
import { ScrollArea as ScrollAreaPrimitive } from "radix-ui";
import { AgentStateBadge } from "@/components/ui/agent-state-badge";
import { TabBarItem } from "@/components/ui/tab-bar";
import { useTabDrag } from "@/hooks/useTabDrag";
import { cn } from "@/lib/utils";
import type { WorkspaceTab } from "@/types/workspace-tab";

export interface WorkspaceTabBarProps {
	tabs: WorkspaceTab[];
	activeTabId: string;
	onTabClick: (id: string) => void;
	onTabClose: (id: string) => void;
	onReorderTabs?: (fromId: string, toId: string) => void;
}

export function WorkspaceTabBar({
	tabs,
	activeTabId,
	onTabClick,
	onTabClose,
	onReorderTabs,
}: WorkspaceTabBarProps) {
	const { dragHandlers, draggingId, dropTarget } = useTabDrag(
		onReorderTabs ?? (() => {}),
	);

	const worktreeTabs = tabs.filter((t) => t.type === "worktree");
	const distinctRepoNames = new Set(
		worktreeTabs
			.map((t) => (t.type === "worktree" ? t.repoName : undefined))
			.filter(Boolean),
	);
	const showRepoPrefix = distinctRepoNames.size > 1;

	const kanbanTab = tabs.find((t) => t.type === "kanban");
	const kanbanHandlers = kanbanTab
		? dragHandlers({ tabId: kanbanTab.id, isDraggable: false })
		: undefined;
	const kanbanDropLeft =
		kanbanTab &&
		dropTarget?.tabId === kanbanTab.id &&
		dropTarget.position === "left";
	const kanbanDropRight =
		kanbanTab &&
		dropTarget?.tabId === kanbanTab.id &&
		dropTarget.position === "right";

	return (
		<ScrollAreaPrimitive.Root className="h-[34px] bg-sidebar border-b border-border shrink-0">
			<ScrollAreaPrimitive.Viewport className="h-full w-full">
				<div className="flex items-center h-[34px]">
					{kanbanTab && (
						<button
							type="button"
							className={cn(
								"flex items-center gap-2 h-full px-3 text-sm border-r border-border cursor-pointer transition-colors shrink-0",
								activeTabId === kanbanTab.id
									? "bg-background text-foreground"
									: "bg-sidebar text-muted-foreground hover:bg-sidebar-accent",
								kanbanDropLeft && "border-l-2 border-l-primary",
								kanbanDropRight && "border-r-2 border-r-primary",
							)}
							onClick={() => onTabClick(kanbanTab.id)}
							aria-label="Kanban"
							onDragOver={kanbanHandlers?.onDragOver}
							onDragLeave={kanbanHandlers?.onDragLeave}
							onDrop={kanbanHandlers?.onDrop}
						>
							<LayoutGrid className="size-4 shrink-0" />
						</button>
					)}

					<div
						role="tablist"
						aria-orientation="horizontal"
						className="flex items-center h-full"
					>
						{worktreeTabs.map((tab) => {
							const isActive = tab.id === activeTabId;
							const isDragging = draggingId === tab.id;
							const isDropLeft =
								dropTarget?.tabId === tab.id && dropTarget.position === "left";
							const isDropRight =
								dropTarget?.tabId === tab.id && dropTarget.position === "right";
							const handlers = dragHandlers({
								tabId: tab.id,
								isDraggable: true,
							});

							return (
								<TabBarItem
									key={tab.id}
									isActive={isActive}
									onClick={() => onTabClick(tab.id)}
									onClose={(e) => {
										e.stopPropagation();
										onTabClose(tab.id);
									}}
									closeLabel={`Close ${showRepoPrefix && tab.repoName ? `${tab.repoName} / ${tab.branchName}` : tab.branchName}`}
									className={cn(
										isDragging && "opacity-50",
										isDropLeft && "border-l-2 border-l-primary",
										isDropRight && "border-r-2 border-r-primary",
									)}
									draggable={handlers.draggable}
									onDragStart={handlers.onDragStart}
									onDragEnd={handlers.onDragEnd}
									onDragOver={handlers.onDragOver}
									onDragLeave={handlers.onDragLeave}
									onDrop={handlers.onDrop}
								>
									<GitBranch className="size-4 shrink-0" />
									<span className="truncate max-w-40">
										{showRepoPrefix && tab.repoName
											? `${tab.repoName} / ${tab.branchName}`
											: tab.branchName}
									</span>
									{tab.agentState && (
										<AgentStateBadge state={tab.agentState} variant="dot" />
									)}
								</TabBarItem>
							);
						})}
					</div>
				</div>
			</ScrollAreaPrimitive.Viewport>
			<ScrollAreaPrimitive.Scrollbar
				orientation="horizontal"
				className="flex h-1 touch-none select-none flex-col"
			>
				<ScrollAreaPrimitive.Thumb className="relative flex-1 rounded-full bg-muted-foreground/50" />
			</ScrollAreaPrimitive.Scrollbar>
		</ScrollAreaPrimitive.Root>
	);
}
