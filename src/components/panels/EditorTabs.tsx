import { ScrollArea as ScrollAreaPrimitive } from "radix-ui";
import { FileIcon } from "@react-symbols/icons/utils";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { TabInfo } from "@/types/editor";

export interface EditorTabsProps {
	tabs: TabInfo[];
	activeTabPath: string | null;
	onTabClick: (path: string) => void;
	onTabClose: (path: string) => void;
}

export function EditorTabs({
	tabs,
	activeTabPath,
	onTabClick,
	onTabClose,
}: EditorTabsProps) {
	return (
		<ScrollAreaPrimitive.Root className="h-[30px] bg-sidebar border-b border-border">
			<ScrollAreaPrimitive.Viewport className="h-full w-full">
				<div
					className="flex items-center h-[30px]"
					role="tablist"
					aria-orientation="horizontal"
				>
					{tabs.map((tab) => {
						const isActive = tab.path === activeTabPath;
						return (
							<div
								key={tab.path}
								className={cn(
									"group flex items-center gap-2 h-full px-3 text-sm border-r border-border cursor-pointer transition-colors shrink-0",
									isActive
										? "bg-background text-foreground"
										: "bg-sidebar text-muted-foreground hover:bg-sidebar-accent",
								)}
								onClick={() => onTabClick(tab.path)}
								onKeyDown={(e) => {
									if (e.key === "Enter" || e.key === " ") {
										e.preventDefault();
										onTabClick(tab.path);
									}
								}}
								role="tab"
								tabIndex={0}
								aria-selected={isActive}
							>
								<FileIcon fileName={tab.name} className="h-4 w-4 shrink-0" />
								<span className="truncate max-w-32">{tab.name}</span>
								{tab.isDirty && (
									<span className="w-2 h-2 rounded-full bg-foreground shrink-0" />
								)}
								<button
									type="button"
									onClick={(e) => {
										e.stopPropagation();
										onTabClose(tab.path);
									}}
									className={cn(
										"p-0.5 rounded hover:bg-muted-foreground/20 transition-colors shrink-0",
										isActive
											? "opacity-100"
											: "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
									)}
									aria-label={`Close ${tab.name}`}
								>
									<X className="h-3.5 w-3.5" />
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
