import type { LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { TabsList, TabsTrigger } from "@/components/ui/tabs";
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
	rightSlot?: React.ReactNode;
}

export function ViewToolbar({ leftPanels, rightSlot }: ViewToolbarProps) {
	return (
		<div
			data-tauri-drag-region
			className={cn(
				"flex items-center h-[34px] pl-0 pr-[12px] border-b border-border bg-sidebar shrink-0 gap-0.5",
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
			<TabsList variant="line" className="h-[30px] px-0">
				<TabsTrigger value="agent" className="text-xs px-[10px] py-1">
					Agent
				</TabsTrigger>
				<TabsTrigger value="editor" className="text-xs px-[10px] py-1">
					Editor
				</TabsTrigger>
			</TabsList>
			<div data-tauri-drag-region className="flex-1" />
			{rightSlot}
		</div>
	);
}
