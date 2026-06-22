import type { LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export interface TogglePanel {
	id: string;
	icon: LucideIcon;
	label: string;
	visible: boolean;
	onToggle: () => void;
}

interface ViewToolbarProps {
	leftPanels?: TogglePanel[];
	centerSlot?: React.ReactNode;
	edgePadding?: "default" | "grid";
	rightSlot?: React.ReactNode;
}

export function ViewToolbar({
	leftPanels,
	centerSlot,
	edgePadding = "default",
	rightSlot,
}: ViewToolbarProps) {
	return (
		<div
			data-tauri-drag-region
			className={cn(
				"flex items-center h-[34px] pl-0 border-b border-border bg-sidebar shrink-0 gap-0.5",
				edgePadding === "grid" ? "pr-2" : "pr-[12px]",
				leftPanels && leftPanels.length > 0 && "pl-[80px]",
			)}
		>
			{leftPanels?.map((panel) => (
				<Tooltip key={panel.id}>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className={cn(
								"h-6 w-6",
								panel.visible ? "text-foreground" : "text-muted-foreground",
							)}
							onClick={panel.onToggle}
							aria-label={`Toggle ${panel.label}`}
						>
							<panel.icon className="size-4" />
						</Button>
					</TooltipTrigger>
					<TooltipContent side="bottom">{panel.label}</TooltipContent>
				</Tooltip>
			))}
			{centerSlot ? (
				<div className="min-w-0 flex-1">{centerSlot}</div>
			) : (
				<div data-tauri-drag-region className="flex-1" />
			)}
			{rightSlot}
		</div>
	);
}
