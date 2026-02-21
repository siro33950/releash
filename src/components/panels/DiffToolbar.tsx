import {
	AlignJustify,
	ChevronLeft,
	ChevronRight,
	Minus,
	SplitSquareHorizontal,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { DiffBase, DiffMode } from "@/types/settings";

export interface DiffToolbarProps {
	diffBase: DiffBase;
	diffMode: DiffMode;
	currentIndex: number;
	total: number;
	onDiffBaseChange: (base: DiffBase) => void;
	onDiffModeChange: (mode: DiffMode) => void;
	onGoToPrev: () => void;
	onGoToNext: () => void;
	onStageAll: () => void;
	onUnstageAll: () => void;
	showStageButtons: boolean;
}

export function DiffToolbar({
	diffBase,
	diffMode,
	currentIndex,
	total,
	onDiffBaseChange,
	onDiffModeChange,
	onGoToPrev,
	onGoToNext,
	onStageAll,
	onUnstageAll,
	showStageButtons,
}: DiffToolbarProps) {
	return (
		<div className="flex items-center justify-between px-3 py-1.5 border-t border-border bg-card">
			<div className="flex items-center gap-2">
				<span className="text-xs text-muted-foreground">Base:</span>
				<select
					value={diffBase}
					onChange={(e) => onDiffBaseChange(e.target.value as DiffBase)}
					className="bg-muted border border-border rounded px-2 py-0.5 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
				>
					<option value="HEAD">HEAD</option>
					<option value="staged">Staged</option>
				</select>
				{total > 0 && (
					<div className="flex items-center gap-1 ml-2">
						{showStageButtons && (
							<>
								<button
									type="button"
									onClick={onStageAll}
									className="px-1.5 py-0.5 text-[10px] bg-status-added/20 text-status-added rounded hover:bg-status-added/30 transition-colors"
								>
									Stage All
								</button>
								{diffBase === "HEAD" && (
									<button
										type="button"
										onClick={onUnstageAll}
										className="px-1.5 py-0.5 text-[10px] bg-status-modified/20 text-status-modified rounded hover:bg-status-modified/30 transition-colors"
									>
										Unstage All
									</button>
								)}
							</>
						)}
						<button
							type="button"
							onClick={onGoToPrev}
							className="p-0.5 rounded hover:bg-muted transition-colors text-muted-foreground hover:text-foreground"
							title="Previous hunk"
						>
							<ChevronLeft className="h-3.5 w-3.5" />
						</button>
						<button
							type="button"
							onClick={onGoToNext}
							className="p-0.5 rounded hover:bg-muted transition-colors text-muted-foreground hover:text-foreground"
							title="Next hunk"
						>
							<ChevronRight className="h-3.5 w-3.5" />
						</button>
						<span className="text-[10px] text-muted-foreground font-mono">
							{currentIndex + 1}/{total}
						</span>
					</div>
				)}
			</div>
			<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
				<button
					type="button"
					onClick={() => onDiffModeChange("gutter")}
					className={cn(
						"flex items-center gap-1 px-2 py-0.5 rounded text-xs transition-colors",
						diffMode === "gutter"
							? "bg-background shadow-sm text-foreground"
							: "text-muted-foreground hover:text-foreground",
					)}
					title="Gutter markers only"
				>
					<Minus className="h-3.5 w-3.5" />
					Gutter
				</button>
				<button
					type="button"
					onClick={() => onDiffModeChange("inline")}
					className={cn(
						"flex items-center gap-1 px-2 py-0.5 rounded text-xs transition-colors",
						diffMode === "inline"
							? "bg-background shadow-sm text-foreground"
							: "text-muted-foreground hover:text-foreground",
					)}
					title="Inline diff"
				>
					<AlignJustify className="h-3.5 w-3.5" />
					Inline
				</button>
				<button
					type="button"
					onClick={() => onDiffModeChange("split")}
					className={cn(
						"flex items-center gap-1 px-2 py-0.5 rounded text-xs transition-colors",
						diffMode === "split"
							? "bg-background shadow-sm text-foreground"
							: "text-muted-foreground hover:text-foreground",
					)}
					title="Split view"
				>
					<SplitSquareHorizontal className="h-3.5 w-3.5" />
					Split
				</button>
			</div>
		</div>
	);
}
