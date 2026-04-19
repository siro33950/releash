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
import type { FileNavigationResult } from "@/hooks/useFileNavigation";
import { cn } from "@/lib/utils";
import type { DiffMode } from "@/types/settings";

export interface DiffToolbarProps {
	diffMode: DiffMode;
	onDiffModeChange: (mode: DiffMode) => void;
	fileNavigation?: FileNavigationResult;
	onGoToPrevFile?: () => void;
	onGoToNextFile?: () => void;
}

const diffModes: { mode: DiffMode; icon: typeof Minus; label: string }[] = [
	{ mode: "gutter", icon: Minus, label: "Gutter" },
	{ mode: "inline", icon: AlignJustify, label: "Inline" },
	{ mode: "split", icon: SplitSquareHorizontal, label: "Split" },
];

export function DiffToolbar({
	diffMode,
	onDiffModeChange,
	fileNavigation,
	onGoToPrevFile,
	onGoToNextFile,
}: DiffToolbarProps) {
	const hasFileNav = fileNavigation && fileNavigation.total > 0;

	return (
		<div className="flex items-center justify-between px-2 h-[36px] border-t border-border bg-card shrink-0 select-none">
			{/* Left: spacer */}
			<div className="flex items-center h-full" />

			{/* Center: File navigation */}
			{hasFileNav && (
				<div className="flex items-center gap-1">
					<Button
						variant="ghost"
						size="icon-xs"
						onClick={onGoToPrevFile}
						disabled={fileNavigation.prev_file === null}
						title="Previous file"
						aria-label="Previous file"
						className="text-muted-foreground hover:text-foreground disabled:opacity-30"
					>
						<ChevronLeft className="h-3.5 w-3.5" />
					</Button>
					<Button
						variant="ghost"
						size="icon-xs"
						onClick={onGoToNextFile}
						disabled={fileNavigation.next_file === null}
						title="Next file"
						aria-label="Next file"
						className="text-muted-foreground hover:text-foreground disabled:opacity-30"
					>
						<ChevronRight className="h-3.5 w-3.5" />
					</Button>
					<span className="text-[10px] text-muted-foreground font-mono">
						{fileNavigation.current_index + 1}/{fileNavigation.total}
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
