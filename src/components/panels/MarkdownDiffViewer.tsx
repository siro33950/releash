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
import { getErrorMessage } from "@/lib/errorMessage";
import { rehypeSourceLines } from "@/lib/rehypeSourceLines";
import type { DiffRange, InlineChunk, SplitRow } from "@/types/markdown-diff";
import type { DiffMode } from "@/types/settings";

interface VisibleBlock {
	startLine: number;
	endLine: number;
	content: string;
	deletedContent?: string;
}

interface ReadModelArgs {
	original: string;
	modified: string;
	[key: string]: unknown;
}

interface ReadModelInputKey {
	original: string;
	modified: string;
}

interface StoredReadModel<T> {
	inputKey: ReadModelInputKey;
	data: T;
}

type StoredReadModelState<T> =
	| { status: "loading"; result: StoredReadModel<T> | null; error: null }
	| { status: "ready"; result: StoredReadModel<T>; error: null }
	| { status: "error"; result: StoredReadModel<T> | null; error: string };

type ReadModelState<T> =
	| { status: "loading"; data: T; error: null }
	| { status: "ready"; data: T; error: null }
	| { status: "error"; data: T; error: string };

const EMPTY_DIFF_RANGES: DiffRange[] = [];
const EMPTY_SPLIT_ROWS: SplitRow[] = [];
const EMPTY_INLINE_CHUNKS: InlineChunk[] = [];
const EMPTY_VISIBLE_BLOCKS: VisibleBlock[] = [];

export interface MarkdownDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	diffMode?: DiffMode;
	diffOnlyMode?: boolean;
}

const remarkPlugins = [remarkGfm];

function readModelInputKeyFromArgs(args: ReadModelArgs): ReadModelInputKey {
	return {
		original: args.original,
		modified: args.modified,
	};
}

function readModelInputKeysEqual(
	left: ReadModelInputKey,
	right: ReadModelInputKey,
): boolean {
	return left.original === right.original && left.modified === right.modified;
}

function currentReadModelResult<T>(
	result: StoredReadModel<T> | null,
	inputKey: ReadModelInputKey,
): StoredReadModel<T> | null {
	return result && readModelInputKeysEqual(result.inputKey, inputKey)
		? result
		: null;
}

function useReadModel<T>(
	command: string,
	args: ReadModelArgs,
	fallbackData: T,
): ReadModelState<T> {
	const inputKey = useMemo(() => readModelInputKeyFromArgs(args), [args]);
	const [state, setState] = useState<StoredReadModelState<T>>({
		status: "loading",
		result: null,
		error: null,
	});

	useEffect(() => {
		let cancelled = false;
		setState((prev) => ({
			status: "loading",
			result: currentReadModelResult(prev.result, inputKey),
			error: null,
		}));
		invoke<T>(command, args)
			.then((data) => {
				if (!cancelled) {
					setState({
						status: "ready",
						result: { inputKey, data },
						error: null,
					});
				}
			})
			.catch((error: unknown) => {
				if (!cancelled) {
					setState((prev) => ({
						status: "error",
						result: currentReadModelResult(prev.result, inputKey),
						error: getErrorMessage(error),
					}));
				}
			});

		return () => {
			cancelled = true;
		};
	}, [command, args, inputKey]);

	const result = currentReadModelResult(state.result, inputKey);
	const data = result?.data ?? fallbackData;
	if (state.status === "error") {
		return { status: "error", data, error: state.error };
	}
	if (state.status === "ready") {
		return { status: "ready", data, error: null };
	}
	return { status: "loading", data, error: null };
}

function ReadModelErrorNotice({ message }: { message: string }) {
	return (
		<div
			role="alert"
			className="m-4 rounded border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
		>
			{message}
		</div>
	);
}

function GutterView({
	modifiedContent,
	originalContent,
}: {
	modifiedContent: string;
	originalContent: string;
}) {
	const readModelArgs = useMemo(
		() => ({
			original: originalContent,
			modified: modifiedContent,
			side: "modified",
		}),
		[originalContent, modifiedContent],
	);
	const diffRanges = useReadModel<DiffRange[]>(
		"compute_markdown_diff_ranges",
		readModelArgs,
		EMPTY_DIFF_RANGES,
	);

	const rehypePlugins = useMemo(
		() => [rehypeSourceLines(diffRanges.data), rehypeHighlight],
		[diffRanges.data],
	);
	return (
		<div className="h-full overflow-auto">
			{diffRanges.status === "error" && (
				<ReadModelErrorNotice message={diffRanges.error} />
			)}
			<div className="markdown-preview p-6">
				<Markdown remarkPlugins={remarkPlugins} rehypePlugins={rehypePlugins}>
					{modifiedContent}
				</Markdown>
			</div>
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
	const readModelArgs = useMemo(
		() => ({
			original: originalContent,
			modified: modifiedContent,
		}),
		[originalContent, modifiedContent],
	);
	const rows = useReadModel<SplitRow[]>(
		"compute_markdown_split_rows",
		readModelArgs,
		EMPTY_SPLIT_ROWS,
	);

	const rehypePlugins = useMemo(() => [rehypeHighlight], []);

	return (
		<div className="md-split-container" data-testid="md-split-grid">
			{rows.status === "error" && <ReadModelErrorNotice message={rows.error} />}
			{rows.data.map((row, rowIndex) => (
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
	const readModelArgs = useMemo(
		() => ({
			original: originalContent,
			modified: modifiedContent,
		}),
		[originalContent, modifiedContent],
	);
	const chunks = useReadModel<InlineChunk[]>(
		"compute_markdown_inline_chunks",
		readModelArgs,
		EMPTY_INLINE_CHUNKS,
	);

	const rehypePlugins = useMemo(() => [rehypeHighlight], []);
	return (
		<div className="h-full overflow-auto">
			{chunks.status === "error" && (
				<ReadModelErrorNotice message={chunks.error} />
			)}
			<div className="markdown-preview p-6">
				{chunks.data.map((chunk, chunkIndex) => {
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
		</div>
	);
}

function DiffOnlyMarkdownView({
	originalContent,
	modifiedContent,
}: {
	originalContent: string;
	modifiedContent: string;
}) {
	const [expandedGaps, setExpandedGaps] = useState<Set<number>>(new Set());
	const rehypePlugins = useMemo(() => [rehypeHighlight], []);
	const readModelArgs = useMemo(
		() => ({
			original: originalContent,
			modified: modifiedContent,
			contextLines: 3,
		}),
		[originalContent, modifiedContent],
	);
	const visibleBlocks = useReadModel<VisibleBlock[]>(
		"compute_visible_markdown_blocks",
		readModelArgs,
		EMPTY_VISIBLE_BLOCKS,
	);

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

	const blocks = visibleBlocks.data;

	if (visibleBlocks.status === "error" && blocks.length === 0) {
		return <ReadModelErrorNotice message={visibleBlocks.error} />;
	}

	if (blocks.length === 0) {
		return (
			<div className="h-full flex items-center justify-center text-muted-foreground text-sm">
				No changes
			</div>
		);
	}

	const lastBlock = blocks[blocks.length - 1];
	const trailingGapLines = lastBlock ? modLines.length - lastBlock.endLine : 0;
	const trailingGapIndex = blocks.length;

	return (
		<div className="markdown-preview h-full overflow-auto p-6">
			{visibleBlocks.status === "error" && (
				<ReadModelErrorNotice message={visibleBlocks.error} />
			)}
			{blocks.map((block, i) => {
				const prevEnd = blocks[i - 1]?.endLine ?? 0;
				const gapLines = block.startLine - prevEnd - 1;

				return (
					// biome-ignore lint/suspicious/noArrayIndexKey: blocks are positional, order is fixed
					<div key={i}>
						{gapLines > 0 &&
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
						{block.deletedContent && (
							<div className="border-l-2 border-red-500/50 pl-3 my-2 opacity-70">
								<div className="text-xs text-red-400 mb-1">Deleted:</div>
								<div className="line-through text-muted-foreground">
									<Markdown
										remarkPlugins={remarkPlugins}
										rehypePlugins={rehypePlugins}
									>
										{block.deletedContent}
									</Markdown>
								</div>
							</div>
						)}
						<Markdown
							remarkPlugins={remarkPlugins}
							rehypePlugins={rehypePlugins}
						>
							{block.content}
						</Markdown>
					</div>
				);
			})}
			{trailingGapLines > 0 &&
				(expandedGaps.has(trailingGapIndex) ? (
					<div className="border-y border-border my-4 py-2 opacity-60">
						<Markdown
							remarkPlugins={remarkPlugins}
							rehypePlugins={rehypePlugins}
						>
							{modLines.slice(lastBlock.endLine).join("\n")}
						</Markdown>
					</div>
				) : (
					<button
						type="button"
						onClick={() => expandGap(trailingGapIndex)}
						className="flex w-full items-center justify-center text-xs text-muted-foreground py-2 border-y border-border my-4 cursor-pointer hover:bg-muted/50"
					>
						··· {trailingGapLines} lines hidden ···
					</button>
				))}
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
					key={`${deferredOriginal}\0${deferredModified}`}
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
