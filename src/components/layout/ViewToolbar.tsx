import { type LucideIcon, MessageSquare, Workflow } from "lucide-react";
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

/**
 * spec issues-1023: 中央エリアの表示モード。
 * Agent は既存の AgentChat、Workflow は workflow run 観測の Command Center。
 */
export type CenterMode = "agent" | "workflow";

const MODES: { value: CenterMode; label: string; icon: LucideIcon }[] = [
	{ value: "agent", label: "Agent", icon: MessageSquare },
	{ value: "workflow", label: "Workflow", icon: Workflow },
];

interface ViewToolbarProps {
	leftPanels?: TogglePanel[];
	rightSlot?: React.ReactNode;
	mode?: CenterMode;
	onModeChange?: (mode: CenterMode) => void;
}

export function ViewToolbar({
	leftPanels,
	rightSlot,
	mode,
	onModeChange,
}: ViewToolbarProps) {
	const showModeSwitch = mode !== undefined && onModeChange !== undefined;
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
			{showModeSwitch && (
				<div
					data-testid="center-mode-switch"
					className="flex items-center gap-0.5 rounded bg-muted p-0.5 ml-1"
				>
					{MODES.map((m) => {
						const Icon = m.icon;
						const active = mode === m.value;
						return (
							<Tooltip key={m.value}>
								<TooltipTrigger asChild>
									<Button
										variant="ghost"
										size="icon"
										aria-label={`${m.label} mode`}
										aria-pressed={active}
										onClick={() => onModeChange?.(m.value)}
										className={cn(
											"h-5 w-6",
											active
												? "bg-background text-foreground shadow-sm"
												: "text-muted-foreground hover:text-foreground",
										)}
									>
										<Icon className="size-3.5" />
									</Button>
								</TooltipTrigger>
								<TooltipContent side="bottom">{m.label}</TooltipContent>
							</Tooltip>
						);
					})}
				</div>
			)}
			<div data-tauri-drag-region className="flex-1" />
			{rightSlot}
		</div>
	);
}
