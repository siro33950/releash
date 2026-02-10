import type { ReactNode } from "react";

interface KanbanColumnProps {
	icon: ReactNode;
	title: string;
	count: number;
	children: ReactNode;
}

export function KanbanColumn({
	icon,
	title,
	count,
	children,
}: KanbanColumnProps) {
	return (
		<div className="flex flex-col min-h-0 rounded-lg border border-border bg-card/30">
			<div className="flex items-center gap-2 px-3 py-2 border-b border-border shrink-0">
				{icon}
				<span className="text-xs font-semibold">{title}</span>
				<span className="ml-auto text-xs text-muted-foreground">{count}</span>
			</div>
			<div className="flex-1 overflow-y-auto p-2 space-y-2">{children}</div>
		</div>
	);
}
