import { invoke } from "@tauri-apps/api/core";
import {
	AlignJustify,
	ChevronLeft,
	ChevronRight,
	Diff,
	ExternalLink,
	Minus,
	SplitSquareHorizontal,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
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
	diffOnlyMode: boolean;
	onDiffModeChange: (mode: DiffMode) => void;
	onDiffOnlyModeChange: (enabled: boolean) => void;
	fileNavigation?: FileNavigationResult;
	onGoToPrevFile?: () => void;
	onGoToNextFile?: () => void;
	filePath: string | null;
}

const diffModes: { mode: DiffMode; icon: typeof Minus; label: string }[] = [
	{ mode: "gutter", icon: Minus, label: "Gutter" },
	{ mode: "inline", icon: AlignJustify, label: "Inline" },
	{ mode: "split", icon: SplitSquareHorizontal, label: "Split" },
];

export function DiffToolbar({
	diffMode,
	diffOnlyMode,
	onDiffModeChange,
	onDiffOnlyModeChange,
	fileNavigation,
	onGoToPrevFile,
	onGoToNextFile,
	filePath,
}: DiffToolbarProps) {
	const hasFileNav = fileNavigation && fileNavigation.total > 0;
	const [editorError, setEditorError] = useState<string | null>(null);
	const errorTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	useEffect(() => {
		return () => {
			if (errorTimerRef.current) clearTimeout(errorTimerRef.current);
		};
	}, []);

	const handleOpenInEditor = useCallback(() => {
		if (!filePath) return;
		setEditorError(null);
		invoke("open_in_editor", { filePath }).catch((e) => {
			console.error("Failed to open in editor:", e);
			setEditorError(String(e));
			if (errorTimerRef.current) clearTimeout(errorTimerRef.current);
			errorTimerRef.current = setTimeout(() => setEditorError(null), 5000);
		});
	}, [filePath]);

	return (
		<div className="flex items-center justify-between px-2 h-[36px] border-t border-border bg-card shrink-0 select-none">
			{/* Left: Open in Editor */}
			<div className="flex items-center h-full gap-1">
				{filePath && (
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon-xs"
								aria-label="Open in Editor"
								onClick={handleOpenInEditor}
								className="h-5 w-5 text-muted-foreground hover:text-foreground"
							>
								<ExternalLink className="h-3.5 w-3.5" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="top" className="text-xs">
							Open in Editor
						</TooltipContent>
					</Tooltip>
				)}
				{editorError && (
					<span className="text-[10px] text-destructive truncate max-w-[200px]">
						{editorError}
					</span>
				)}
			</div>

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

			{/* Right: Diff-only toggle + Diff mode icons */}
			<div className="flex items-center gap-1">
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon-xs"
							aria-label="Diff only"
							aria-pressed={diffOnlyMode}
							onClick={() => onDiffOnlyModeChange(!diffOnlyMode)}
							className={cn(
								"w-6 h-5",
								diffOnlyMode
									? "bg-primary/20 text-primary"
									: "text-muted-foreground hover:text-foreground",
							)}
						>
							<Diff className="h-3.5 w-3.5" />
						</Button>
					</TooltipTrigger>
					<TooltipContent side="top" className="text-xs">
						{diffOnlyMode ? "Show full file" : "Show diff only"}
					</TooltipContent>
				</Tooltip>

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
