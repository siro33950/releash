import { GitBranch, LayoutGrid, X } from "lucide-react";
import { ScrollArea as ScrollAreaPrimitive } from "radix-ui";
import { cn } from "@/lib/utils";
import type { AgentState } from "@/types/protocol";
import type { WorkspaceTab } from "@/types/workspace-tab";

export interface WorkspaceTabBarProps {
	tabs: WorkspaceTab[];
	activeTabId: string;
	onTabClick: (id: string) => void;
	onTabClose: (id: string) => void;
}

const agentStateColor: Record<AgentState, string> = {
	running: "bg-blue-500 animate-pulse",
	waiting: "bg-yellow-500 animate-pulse",
	done: "bg-green-500",
	error: "bg-red-500",
};

export function WorkspaceTabBar({
	tabs,
	activeTabId,
	onTabClick,
	onTabClose,
}: WorkspaceTabBarProps) {
	const worktreeTabs = tabs.filter((t) => t.type === "worktree");
	const distinctRepoNames = new Set(
		worktreeTabs
			.map((t) => (t.type === "worktree" ? t.repoName : undefined))
			.filter(Boolean),
	);
	const showRepoPrefix = distinctRepoNames.size > 1;

	return (
		<ScrollAreaPrimitive.Root className="h-[34px] bg-sidebar border-b border-border shrink-0">
			<ScrollAreaPrimitive.Viewport className="h-full w-full">
				<div
					className="flex items-center h-[34px]"
					role="tablist"
					aria-orientation="horizontal"
				>
					{tabs.map((tab) => {
						const isActive = tab.id === activeTabId;
						if (tab.type === "kanban") {
							return (
								<div
									key={tab.id}
									className={cn(
										"flex items-center gap-2 h-full px-3 text-sm border-r border-border cursor-pointer transition-colors shrink-0",
										isActive
											? "bg-background text-foreground"
											: "bg-sidebar text-muted-foreground hover:bg-sidebar-accent",
									)}
									onClick={() => onTabClick(tab.id)}
									onKeyDown={(e) => {
										if (e.key === "Enter" || e.key === " ") {
											e.preventDefault();
											onTabClick(tab.id);
										}
									}}
									role="tab"
									tabIndex={0}
									aria-selected={isActive}
								>
									<LayoutGrid className="size-4 shrink-0" />
									<span>Kanban</span>
								</div>
							);
						}
						return (
							<div
								key={tab.id}
								className={cn(
									"group flex items-center gap-2 h-full px-3 text-sm border-r border-border cursor-pointer transition-colors shrink-0",
									isActive
										? "bg-background text-foreground"
										: "bg-sidebar text-muted-foreground hover:bg-sidebar-accent",
								)}
								onClick={() => onTabClick(tab.id)}
								onKeyDown={(e) => {
									if (e.key === "Enter" || e.key === " ") {
										e.preventDefault();
										onTabClick(tab.id);
									}
								}}
								role="tab"
								tabIndex={0}
								aria-selected={isActive}
							>
								<GitBranch className="size-4 shrink-0" />
								<span className="truncate max-w-40">
									{showRepoPrefix && tab.repoName
										? `${tab.repoName} / ${tab.branchName}`
										: tab.branchName}
								</span>
								{tab.agentState && (
									<span
										className={cn(
											"w-2 h-2 rounded-full shrink-0",
											agentStateColor[tab.agentState],
										)}
										title={tab.agentState}
									/>
								)}
								<button
									type="button"
									onClick={(e) => {
										e.stopPropagation();
										onTabClose(tab.id);
									}}
									className={cn(
										"p-0.5 rounded hover:bg-muted-foreground/20 transition-colors shrink-0",
										isActive
											? "opacity-100"
											: "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
									)}
									aria-label={`Close ${tab.branchName}`}
								>
									<X className="size-3.5" />
								</button>
							</div>
						);
					})}
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
