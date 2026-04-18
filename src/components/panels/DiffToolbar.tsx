import {
	AlignJustify,
	ChevronLeft,
	ChevronRight,
	Minus,
	SplitSquareHorizontal,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { DiffBase, DiffMode } from "@/types/settings";

export interface DiffToolbarProps {
	diffBase: DiffBase;
	diffMode: DiffMode;
	currentIndex: number;
	total: number;
	onDiffModeChange: (mode: DiffMode) => void;
	onGoToPrev: () => void;
	onGoToNext: () => void;
	onStageAll: () => void;
	onUnstageAll: () => void;
	showStageButtons: boolean;
}

const diffModes: { mode: DiffMode; icon: typeof Minus; label: string }[] = [
	{ mode: "gutter", icon: Minus, label: "Gutter" },
	{ mode: "inline", icon: AlignJustify, label: "Inline" },
	{ mode: "split", icon: SplitSquareHorizontal, label: "Split" },
];

export function DiffToolbar({
	diffBase,
	diffMode,
	currentIndex,
	total,
	onDiffModeChange,
	onGoToPrev,
	onGoToNext,
	onStageAll,
	onUnstageAll,
	showStageButtons,
}: DiffToolbarProps) {
	return (
		<div className="flex items-center justify-between px-2 h-[36px] border-t border-border bg-card shrink-0 select-none">
			{/* Left: spacer */}
			<div className="flex items-center h-full" />

			{/* Center: Hunk navigation (only when changes exist) */}
			{total > 0 && (
				<div className="flex items-center gap-1">
					{showStageButtons && (
						<>
							<Button
								variant="ghost"
								size="xs"
								onClick={onStageAll}
								className="bg-status-added/20 text-status-added hover:bg-status-added/30 hover:text-status-added text-[10px]"
							>
								Stage All
							</Button>
							{diffBase === "branch-base" && (
								<Button
									variant="ghost"
									size="xs"
									onClick={onUnstageAll}
									className="bg-status-modified/20 text-status-modified hover:bg-status-modified/30 hover:text-status-modified text-[10px]"
								>
									Unstage All
								</Button>
							)}
						</>
					)}
					<Button
						variant="ghost"
						size="icon-xs"
						onClick={onGoToPrev}
						title="Previous hunk"
						aria-label="Previous hunk"
						className="text-muted-foreground hover:text-foreground"
					>
						<ChevronLeft className="h-3.5 w-3.5" />
					</Button>
					<Button
						variant="ghost"
						size="icon-xs"
						onClick={onGoToNext}
						title="Next hunk"
						aria-label="Next hunk"
						className="text-muted-foreground hover:text-foreground"
					>
						<ChevronRight className="h-3.5 w-3.5" />
					</Button>
					<span className="text-[10px] text-muted-foreground font-mono">
						{currentIndex + 1}/{total}
					</span>
				</div>
			)}

			{/* Right: Diff mode icons */}
			<div className="flex items-center gap-1">
				<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
					{diffModes.map(({ mode, icon: Icon, label }) => (
						<Tooltip key={mode}>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon-xs"
									aria-label={label}
									aria-pressed={diffMode === mode}
									onClick={() => onDiffModeChange(mode)}
									className={cn(
										"w-6 h-5",
										diffMode === mode
											? "bg-background shadow-sm text-foreground"
											: "text-muted-foreground hover:text-foreground",
									)}
								>
									<Icon className="h-3.5 w-3.5" />
								</Button>
							</TooltipTrigger>
							<TooltipContent side="top" className="text-xs">
								{label}
							</TooltipContent>
						</Tooltip>
					))}
				</div>
			</div>
		</div>
	);
}
