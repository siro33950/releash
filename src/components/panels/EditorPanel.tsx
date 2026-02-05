import { invoke } from "@tauri-apps/api/core";
import { AlignJustify, Minus, SplitSquareHorizontal } from "lucide-react";
import { useEffect, useState } from "react";
import { useGitOriginalContent } from "@/hooks/useGitOriginalContent";
import { cn } from "@/lib/utils";
import type { TabInfo } from "@/types/editor";
import { EditorTabs } from "./EditorTabs";
import { EmptyState } from "./EmptyState";
import {
	type DiffBase,
	type DiffMode,
	MonacoDiffViewer,
} from "./MonacoDiffViewer";

export interface EditorPanelProps {
	tabs: TabInfo[];
	activeTab: TabInfo | null;
	onTabClick: (path: string) => void;
	onTabClose: (path: string) => void;
	diffBase: DiffBase;
	diffMode: DiffMode;
	onDiffBaseChange: (base: DiffBase) => void;
	onDiffModeChange: (mode: DiffMode) => void;
	onContentChange?: (path: string, content: string) => void;
}

export function EditorPanel({
	tabs,
	activeTab,
	onTabClick,
	onTabClose,
	diffBase,
	diffMode,
	onDiffBaseChange,
	onDiffModeChange,
	onContentChange,
}: EditorPanelProps) {
	const [branches, setBranches] = useState<string[]>([]);
	const filePath = activeTab?.path ?? null;

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

	const originalContent = useGitOriginalContent(
		filePath,
		diffBase,
		activeTab?.originalContent ?? "",
	);

	if (tabs.length === 0) {
		return <EmptyState />;
	}

	const handleContentChange = activeTab
		? (content: string) => onContentChange?.(activeTab.path, content)
		: undefined;

	return (
		<div className="flex flex-col h-full">
			<EditorTabs
				tabs={tabs}
				activeTabPath={activeTab?.path ?? null}
				onTabClick={onTabClick}
				onTabClose={onTabClose}
			/>
			<div className="flex-1 overflow-hidden">
				{activeTab ? (
					<MonacoDiffViewer
						key={activeTab.path}
						originalContent={originalContent}
						modifiedContent={activeTab.content}
						language={activeTab.language}
						diffMode={diffMode}
						onContentChange={handleContentChange}
					/>
				) : (
					<EmptyState />
				)}
			</div>
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
						{branches.map((branch) => (
							<option key={branch} value={branch}>
								{branch}
							</option>
						))}
					</select>
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
		</div>
	);
}
