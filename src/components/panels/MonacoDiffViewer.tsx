import { useRef } from "react";
import { useMonacoDiffEditor } from "@/hooks/useMonacoDiffEditor";
import { useMonacoGutterEditor } from "@/hooks/useMonacoGutterEditor";
import { cn } from "@/lib/utils";

export type DiffMode = "gutter" | "inline" | "split";
export type DiffBase = "HEAD" | "HEAD~1" | "HEAD~5" | "staged";

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

interface MonacoDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	language?: string;
	className?: string;
	diffMode?: DiffMode;
}

export function MonacoDiffViewer({
	originalContent,
	modifiedContent,
	language = "typescript",
	className,
	diffMode = "split",
}: MonacoDiffViewerProps) {
	return (
		<div className={cn("h-full w-full bg-background", className)}>
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
	);
}
