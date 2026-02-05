import { invoke } from "@tauri-apps/api/core";
import { AlignJustify, Minus, SplitSquareHorizontal } from "lucide-react";
import { useEffect, useState } from "react";
import type { DiffBase, DiffMode } from "@/components/panels/MonacoDiffViewer";
import { cn } from "@/lib/utils";

export interface HeaderBarProps {
	filePath: string | null;
	diffBase: DiffBase;
	diffMode: DiffMode;
	onDiffBaseChange: (base: DiffBase) => void;
	onDiffModeChange: (mode: DiffMode) => void;
}

export function HeaderBar({
	filePath,
	diffBase,
	diffMode,
	onDiffBaseChange,
	onDiffModeChange,
}: HeaderBarProps) {
	const [branches, setBranches] = useState<string[]>([]);

	useEffect(() => {
		if (!filePath) {
			setBranches([]);
			return;
		}

		let cancelled = false;
		invoke<string[]>("list_branches", { filePath })
			.then((result) => {
				if (!cancelled) {
					setBranches(result);
				}
			})
			.catch(() => {
				if (!cancelled) {
					setBranches([]);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [filePath]);

	return (
		<div className="flex items-center justify-between px-3 py-2 border-b border-border bg-card">
			<div className="flex items-center gap-3">
				<div className="flex items-center gap-2">
					<span className="text-xs text-muted-foreground">Base:</span>
					<select
						value={diffBase}
						onChange={(e) => onDiffBaseChange(e.target.value as DiffBase)}
						className="bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
					>
						<option value="HEAD">HEAD</option>
						<option value="staged">Staged</option>
						{branches.map((branch) => (
							<option key={branch} value={branch}>
								{branch}
							</option>
						))}
					</select>
				</div>
				<div className="h-4 w-px bg-border" />
				<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
					<button
						type="button"
						onClick={() => onDiffModeChange("gutter")}
						className={cn(
							"flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors",
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
							"flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors",
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
							"flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors",
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
		</div>
	);
}
