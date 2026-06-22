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
	leftSlot?: React.ReactNode;
}

/**
 * spec issues-1023: 右パネル上部のヘッダー。右パネルは Review 専用。
 * Workflow 表示は中央エリアの ViewToolbar 上で切り替える。
 */
export function RightPanelHeader({ panels, leftSlot }: RightPanelHeaderProps) {
	return (
		<div
			data-tauri-drag-region
			className="flex items-center h-[34px] px-[12px] border-b border-border bg-sidebar shrink-0 gap-0.5"
		>
			{leftSlot && <div className="min-w-0">{leftSlot}</div>}
			<div className="flex-1" />
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
