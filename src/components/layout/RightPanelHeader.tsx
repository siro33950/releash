import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { TogglePanel } from "./ViewToolbar";

interface RightPanelHeaderProps {
	panels: TogglePanel[];
}

export function RightPanelHeader({ panels }: RightPanelHeaderProps) {
	return (
		<div
			data-tauri-drag-region
			className="flex items-center justify-end h-[34px] px-[12px] border-b border-border bg-sidebar shrink-0 gap-0.5"
		>
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
