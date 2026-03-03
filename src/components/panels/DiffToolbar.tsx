import {
	AlignJustify,
	Braces,
	ChevronLeft,
	ChevronRight,
	Minus,
	RotateCw,
	SplitSquareHorizontal,
} from "lucide-react";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { useEditorContext } from "@/contexts/EditorContext";
import type { UseLspReturn } from "@/hooks/useLsp";
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

const lspIconStyle: Record<
	UseLspReturn["status"],
	{ className: string; pulse: boolean }
> = {
	idle: { className: "text-muted-foreground/50", pulse: false },
	downloading: { className: "text-info", pulse: true },
	starting: { className: "text-info", pulse: true },
	running: { className: "text-success", pulse: false },
	restarting: { className: "text-warning", pulse: true },
	error: { className: "text-destructive", pulse: false },
	stopped: { className: "text-destructive", pulse: false },
};

const lspDotColor: Record<
	Exclude<UseLspReturn["status"], "idle">,
	{ dot: string; label: string }
> = {
	downloading: { dot: "bg-info", label: "Downloading..." },
	starting: { dot: "bg-warning", label: "Starting" },
	running: { dot: "bg-success", label: "Running" },
	restarting: { dot: "bg-warning", label: "Restarting" },
	error: { dot: "bg-destructive", label: "Error" },
	stopped: { dot: "bg-destructive", label: "Stopped" },
};

function LspIndicator() {
	const { lspStatus, lspError, lspCrashCount, lspRetryManually } =
		useEditorContext();

	const iconStyle = lspIconStyle[lspStatus];

	return (
		<Popover>
			<PopoverTrigger asChild>
				<button
					type="button"
					className="flex items-center px-1.5 h-full hover:bg-muted-foreground/15 transition-colors"
				>
					<Braces
						className={cn(
							"size-3.5",
							iconStyle.className,
							iconStyle.pulse && "animate-pulse",
						)}
					/>
				</button>
			</PopoverTrigger>
			<PopoverContent side="top" align="start" className="w-56 p-3 text-xs">
				<div className="font-medium text-sm mb-2">Language Server</div>
				{lspStatus === "idle" ? (
					<p className="text-muted-foreground">
						No language server available for this file type
					</p>
				) : (
					<div className="space-y-2">
						<div className="flex items-center gap-2">
							<span
								className={cn(
									"inline-block size-2 rounded-full shrink-0",
									lspDotColor[lspStatus].dot,
								)}
							/>
							<span>{lspDotColor[lspStatus].label}</span>
						</div>
						{lspCrashCount > 0 && (
							<div className="text-muted-foreground">
								Crashes: {lspCrashCount}
							</div>
						)}
						{lspError && (
							<div className="text-muted-foreground break-all">
								Error: {lspError}
							</div>
						)}
						<button
							type="button"
							onClick={lspRetryManually}
							className="flex items-center gap-1.5 px-2 py-1 rounded text-xs bg-muted hover:bg-muted-foreground/15 transition-colors w-full"
						>
							<RotateCw className="size-3" />
							Restart
						</button>
					</div>
				)}
			</PopoverContent>
		</Popover>
	);
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
	onDiffBaseChange,
	onDiffModeChange,
	onGoToPrev,
	onGoToNext,
	onStageAll,
	onUnstageAll,
	showStageButtons,
}: DiffToolbarProps) {
	return (
		<div className="flex items-center justify-between px-2 h-[36px] border-t border-border bg-card shrink-0 select-none">
			{/* Left: LSP Indicator */}
			<div className="flex items-center h-full">
				<LspIndicator />
			</div>

			{/* Center: Hunk navigation (only when changes exist) */}
			{total > 0 && (
				<div className="flex items-center gap-1">
					{showStageButtons && (
						<>
							<button
								type="button"
								onClick={onStageAll}
								className="px-1.5 py-0.5 text-[10px] bg-status-added/20 text-status-added rounded hover:bg-status-added/30 transition-colors"
							>
								Stage All
							</button>
							{diffBase === "branch-base" && (
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

			{/* Right: Base selector + Diff mode icons */}
			<div className="flex items-center gap-1">
				<Select
					value={diffBase}
					onValueChange={(v) => onDiffBaseChange(v as DiffBase)}
				>
					<SelectTrigger
						size="sm"
						className="h-6 border-none bg-transparent shadow-none px-1 text-xs font-mono"
					>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="branch-base">Branch Base</SelectItem>
						<SelectItem value="staged">Staged</SelectItem>
					</SelectContent>
				</Select>

				<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
					{diffModes.map(({ mode, icon: Icon, label }) => (
						<Tooltip key={mode}>
							<TooltipTrigger asChild>
								<button
									type="button"
									onClick={() => onDiffModeChange(mode)}
									className={cn(
										"flex items-center justify-center w-6 h-5 rounded transition-colors",
										diffMode === mode
											? "bg-background shadow-sm text-foreground"
											: "text-muted-foreground hover:text-foreground",
									)}
								>
									<Icon className="h-3.5 w-3.5" />
								</button>
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
