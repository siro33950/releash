import { useDeferredValue, useMemo } from "react";
import Markdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { ScrollArea } from "@/components/ui/scroll-area";
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
		<ScrollArea className="h-full">
			<div className="markdown-preview p-6">
				<Markdown remarkPlugins={remarkPlugins} rehypePlugins={rehypePlugins}>
					{modifiedContent}
				</Markdown>
			</div>
		</ScrollArea>
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
			{rows.map((row, rowIndex) => (
				// biome-ignore lint/suspicious/noArrayIndexKey: rows are positional diff output, order is fixed
				<div key={rowIndex} className="md-split-row">
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
		<ScrollArea className="h-full">
			<div className="markdown-preview p-6">
				{chunks.map((chunk, chunkIndex) => {
					const className =
						chunk.type === "added"
							? "md-diff-inline-added"
							: chunk.type === "removed"
								? "md-diff-inline-removed"
								: undefined;
					return (
						// biome-ignore lint/suspicious/noArrayIndexKey: chunks are positional diff output, order is fixed
						<div key={chunkIndex} className={className}>
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
		</ScrollArea>
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
