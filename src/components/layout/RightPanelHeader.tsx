import { GitPullRequest, Workflow } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { TogglePanel } from "./ViewToolbar";

/**
 * spec issues-1023: 右パネル上半分の表示モード。
 * Review は既存の差分閲覧、Workflow は run 観測の Command Center。
 */
export type RightPanelMode = "review" | "workflow";

interface RightPanelHeaderProps {
	panels: TogglePanel[];
	mode?: RightPanelMode;
	onModeChange?: (mode: RightPanelMode) => void;
}

const MODES: { value: RightPanelMode; label: string; icon: typeof Workflow }[] =
	[
		{ value: "review", label: "Review", icon: GitPullRequest },
		{ value: "workflow", label: "Workflow", icon: Workflow },
	];

export function RightPanelHeader({
	panels,
	mode,
	onModeChange,
}: RightPanelHeaderProps) {
	const showModeSwitch = mode !== undefined && onModeChange !== undefined;
	return (
		<div
			data-tauri-drag-region
			className="flex items-center h-[34px] px-[12px] border-b border-border bg-sidebar shrink-0 gap-0.5"
		>
			{showModeSwitch && (
				<div
					data-testid="right-panel-mode-switch"
					className="flex items-center gap-0.5 rounded bg-muted p-0.5"
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
