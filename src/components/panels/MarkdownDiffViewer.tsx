import { invoke } from "@tauri-apps/api/core";
import {
	useCallback,
	useDeferredValue,
	useEffect,
	useMemo,
	useState,
} from "react";
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

interface VisibleBlock {
	startLine: number;
	endLine: number;
	content: string;
}

export interface MarkdownDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	diffMode?: DiffMode;
	diffOnlyMode?: boolean;
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

function DiffOnlyMarkdownView({
	originalContent,
	modifiedContent,
}: {
	originalContent: string;
	modifiedContent: string;
}) {
	const [visibleBlocks, setVisibleBlocks] = useState<VisibleBlock[]>([]);
	const [expandedGaps, setExpandedGaps] = useState<Set<number>>(new Set());
	const rehypePlugins = useMemo(() => [rehypeHighlight], []);

	useEffect(() => {
		setExpandedGaps(new Set());
		invoke<VisibleBlock[]>("compute_visible_markdown_blocks", {
			original: originalContent,
			modified: modifiedContent,
			contextLines: 3,
		})
			.then(setVisibleBlocks)
			.catch(() => setVisibleBlocks([]));
	}, [originalContent, modifiedContent]);

	const expandGap = useCallback((gapIndex: number) => {
		setExpandedGaps((prev) => {
			const next = new Set(prev);
			next.add(gapIndex);
			return next;
		});
	}, []);

	const modLines = useMemo(
		() => modifiedContent.split("\n"),
		[modifiedContent],
	);

	if (visibleBlocks.length === 0) {
		return (
			<div className="h-full flex items-center justify-center text-muted-foreground text-sm">
				No changes
			</div>
		);
	}

	return (
		<div className="markdown-preview h-full overflow-auto p-6 scrollbar-thin">
			{visibleBlocks.map((block, i) => {
				const prevEnd = visibleBlocks[i - 1]?.endLine ?? 0;
				const gapLines = block.startLine - prevEnd - 1;

				return (
					// biome-ignore lint/suspicious/noArrayIndexKey: blocks are positional, order is fixed
					<div key={i}>
						{i > 0 &&
							gapLines > 0 &&
							(expandedGaps.has(i) ? (
								<div className="border-y border-border my-4 py-2 opacity-60">
									<Markdown
										remarkPlugins={remarkPlugins}
										rehypePlugins={rehypePlugins}
									>
										{modLines.slice(prevEnd, block.startLine - 1).join("\n")}
									</Markdown>
								</div>
							) : (
								<button
									type="button"
									onClick={() => expandGap(i)}
									className="flex w-full items-center justify-center text-xs text-muted-foreground py-2 border-y border-border my-4 cursor-pointer hover:bg-muted/50"
								>
									··· {gapLines} lines hidden ···
								</button>
							))}
						<Markdown
							remarkPlugins={remarkPlugins}
							rehypePlugins={rehypePlugins}
						>
							{block.content}
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
	diffOnlyMode,
}: MarkdownDiffViewerProps) {
	const deferredOriginal = useDeferredValue(originalContent);
	const deferredModified = useDeferredValue(modifiedContent);

	if (diffOnlyMode) {
		return (
			<div data-testid="markdown-diff-viewer" className="h-full">
				<DiffOnlyMarkdownView
					originalContent={deferredOriginal}
					modifiedContent={deferredModified}
				/>
			</div>
		);
	}

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
