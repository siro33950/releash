import { useDeferredValue, useMemo } from "react";
import Markdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import {
	computeInlineChunks,
	computeModifiedDiffRanges,
	computeSplitRows,
	type SplitRow,
} from "@/lib/markdownDiff";
import { rehypeSourceLines } from "@/lib/rehypeSourceLines";
import type { DiffMode } from "@/types/settings";

export interface MarkdownDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	diffMode?: DiffMode;
}

const remarkPlugins = [remarkGfm];

function GutterView({
	modifiedContent,
	originalContent,
}: {
	modifiedContent: string;
	originalContent: string;
}) {
	const diffRanges = useMemo(
		() => computeModifiedDiffRanges(originalContent, modifiedContent),
		[originalContent, modifiedContent],
	);
	const rehypePlugins = useMemo(
		() => [rehypeSourceLines(diffRanges), rehypeHighlight],
		[diffRanges],
	);
	return (
		<div className="markdown-preview h-full overflow-auto p-6 scrollbar-thin">
			<Markdown remarkPlugins={remarkPlugins} rehypePlugins={rehypePlugins}>
				{modifiedContent}
			</Markdown>
		</div>
	);
}

function splitCellClass(
	type: SplitRow["type"],
	side: "left" | "right",
): string {
	const base = "md-split-cell";
	switch (type) {
		case "removed":
			return side === "left"
				? `${base} md-split-cell-deleted`
				: `${base} md-split-cell-empty`;
		case "added":
			return side === "left"
				? `${base} md-split-cell-empty`
				: `${base} md-split-cell-added`;
		case "modified":
			return side === "left"
				? `${base} md-split-cell-deleted`
				: `${base} md-split-cell-added`;
		default:
			return base;
	}
}

function SplitView({
	originalContent,
	modifiedContent,
}: {
	originalContent: string;
	modifiedContent: string;
}) {
	const rows = useMemo(
		() => computeSplitRows(originalContent, modifiedContent),
		[originalContent, modifiedContent],
	);
	const rehypePlugins = useMemo(() => [rehypeHighlight], []);

	return (
		<div
			className="md-split-container scrollbar-thin"
			data-testid="md-split-grid"
		>
			{rows.map((row) => (
				<div
					key={`${row.type}-${row.left ?? ""}-${row.right ?? ""}`}
					className="md-split-row"
				>
					<div className={splitCellClass(row.type, "left")}>
						{row.left != null && (
							<div className="markdown-preview">
								<Markdown
									remarkPlugins={remarkPlugins}
									rehypePlugins={rehypePlugins}
								>
									{row.left}
								</Markdown>
							</div>
						)}
					</div>
					<div className="md-split-separator" />
					<div className={splitCellClass(row.type, "right")}>
						{row.right != null && (
							<div className="markdown-preview">
								<Markdown
									remarkPlugins={remarkPlugins}
									rehypePlugins={rehypePlugins}
								>
									{row.right}
								</Markdown>
							</div>
						)}
					</div>
				</div>
			))}
		</div>
	);
}

function InlineView({
	originalContent,
	modifiedContent,
}: {
	originalContent: string;
	modifiedContent: string;
}) {
	const chunks = useMemo(
		() => computeInlineChunks(originalContent, modifiedContent),
		[originalContent, modifiedContent],
	);
	const rehypePlugins = useMemo(() => [rehypeHighlight], []);
	return (
		<div className="markdown-preview h-full overflow-auto p-6 scrollbar-thin">
			{chunks.map((chunk) => {
				const className =
					chunk.type === "added"
						? "md-diff-inline-added"
						: chunk.type === "removed"
							? "md-diff-inline-removed"
							: undefined;
				return (
					<div
						key={`${chunk.type}-${chunk.content.slice(0, 50)}`}
						className={className}
					>
						<Markdown
							remarkPlugins={remarkPlugins}
							rehypePlugins={rehypePlugins}
						>
							{chunk.content}
						</Markdown>
					</div>
				);
			})}
		</div>
	);
}

export function MarkdownDiffViewer({
	originalContent,
	modifiedContent,
	diffMode = "gutter",
}: MarkdownDiffViewerProps) {
	const deferredOriginal = useDeferredValue(originalContent);
	const deferredModified = useDeferredValue(modifiedContent);

	return (
		<div data-testid="markdown-diff-viewer" className="h-full">
			{diffMode === "split" ? (
				<SplitView
					originalContent={deferredOriginal}
					modifiedContent={deferredModified}
				/>
			) : diffMode === "inline" ? (
				<InlineView
					originalContent={deferredOriginal}
					modifiedContent={deferredModified}
				/>
			) : (
				<GutterView
					originalContent={deferredOriginal}
					modifiedContent={deferredModified}
				/>
			)}
		</div>
	);
}
