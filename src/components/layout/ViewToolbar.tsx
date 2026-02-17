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
	panels: TogglePanel[];
}

export function ViewToolbar({ panels }: ViewToolbarProps) {
	return (
		<div className="flex items-center justify-end h-[30px] px-1 border-b border-border bg-sidebar shrink-0 gap-0.5">
			{panels.map((panel) => (
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
		</div>
	);
}
