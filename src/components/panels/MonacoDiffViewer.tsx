import { AlignJustify, Minus, SplitSquareHorizontal } from "lucide-react";
import { useRef, useState } from "react";
import { useMonacoDiffEditor } from "@/hooks/useMonacoDiffEditor";
import { useMonacoGutterEditor } from "@/hooks/useMonacoGutterEditor";
import { cn } from "@/lib/utils";

export type DiffMode = "gutter" | "inline" | "split";
export type DiffBase = "HEAD" | "HEAD~1" | "HEAD~5" | "staged";

const baseOptions: { value: DiffBase; label: string }[] = [
	{ value: "HEAD", label: "HEAD" },
	{ value: "HEAD~1", label: "HEAD~1" },
	{ value: "HEAD~5", label: "HEAD~5" },
	{ value: "staged", label: "Staged" },
];

interface MonacoDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	language?: string;
	className?: string;
	diffBase?: DiffBase;
	onDiffBaseChange?: (base: DiffBase) => void;
}

function GutterEditor({
	originalContent,
	modifiedContent,
	language,
}: {
	originalContent: string;
	modifiedContent: string;
	language: string;
}) {
	const containerRef = useRef<HTMLDivElement>(null);

	useMonacoGutterEditor(containerRef, {
		originalValue: originalContent,
		modifiedValue: modifiedContent,
		language,
	});

	return <div ref={containerRef} className="h-full w-full" />;
}

function DiffEditor({
	originalContent,
	modifiedContent,
	language,
	renderSideBySide,
}: {
	originalContent: string;
	modifiedContent: string;
	language: string;
	renderSideBySide: boolean;
}) {
	const containerRef = useRef<HTMLDivElement>(null);

	useMonacoDiffEditor(containerRef, {
		originalValue: originalContent,
		modifiedValue: modifiedContent,
		language,
		renderSideBySide,
	});

	return <div ref={containerRef} className="h-full w-full" />;
}

export function MonacoDiffViewer({
	originalContent,
	modifiedContent,
	language = "typescript",
	className,
	diffBase: controlledDiffBase,
	onDiffBaseChange,
}: MonacoDiffViewerProps) {
	const [internalDiffBase, setInternalDiffBase] = useState<DiffBase>("HEAD");
	const [diffMode, setDiffMode] = useState<DiffMode>("split");

	const diffBase = controlledDiffBase ?? internalDiffBase;
	const handleDiffBaseChange = (base: DiffBase) => {
		if (onDiffBaseChange) {
			onDiffBaseChange(base);
		} else {
			setInternalDiffBase(base);
		}
	};

	return (
		<div className={cn("flex flex-col h-full bg-background", className)}>
			<div className="flex items-center justify-between px-3 py-2 border-b border-border bg-card">
				<div className="flex items-center gap-3">
					<div className="flex items-center gap-2">
						<span className="text-xs text-muted-foreground">Base:</span>
						<select
							value={diffBase}
							onChange={(e) => handleDiffBaseChange(e.target.value as DiffBase)}
							className="bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
						>
							{baseOptions.map((opt) => (
								<option key={opt.value} value={opt.value}>
									{opt.label}
								</option>
							))}
						</select>
					</div>
					<div className="h-4 w-px bg-border" />
					<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
						<button
							type="button"
							onClick={() => setDiffMode("gutter")}
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
							onClick={() => setDiffMode("inline")}
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
							onClick={() => setDiffMode("split")}
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
			<div className="flex-1 overflow-hidden">
				{diffMode === "gutter" && (
					<GutterEditor
						originalContent={originalContent}
						modifiedContent={modifiedContent}
						language={language}
					/>
				)}
				{diffMode === "inline" && (
					<DiffEditor
						originalContent={originalContent}
						modifiedContent={modifiedContent}
						language={language}
						renderSideBySide={false}
					/>
				)}
				{diffMode === "split" && (
					<DiffEditor
						originalContent={originalContent}
						modifiedContent={modifiedContent}
						language={language}
						renderSideBySide={true}
					/>
				)}
			</div>
		</div>
	);
}
