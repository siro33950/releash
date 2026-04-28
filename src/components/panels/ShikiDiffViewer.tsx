import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke } from "@tauri-apps/api/core";
import { Plus } from "lucide-react";
import React, {
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area";
import { type SearchMatch, useDiffSearch } from "@/hooks/useDiffSearch";
import {
	assignChangeGroupsToBlocks,
	computeDiffBlocks,
	type DiffBlock,
	type DiffLine,
} from "@/hooks/useDiffTokens";
import { useShikiHighlighter } from "@/hooks/useShikiHighlighter";
import type { ChangeGroup, Hunk } from "@/lib/computeHunks";
import type { DiffComment } from "@/types/diffComment";
import type { DiffMode } from "@/types/settings";
import { DiffInlineComment, DiffInlineCommentInput } from "./DiffInlineComment";
import { DiffSearchBar } from "./DiffSearchBar";

interface HiddenRange {
	startLine: number;
	endLine: number;
	hiddenCount: number;
}

function findHiddenRange(
	lineNum: number,
	ranges: HiddenRange[],
): HiddenRange | undefined {
	let lo = 0;
	let hi = ranges.length - 1;
	while (lo <= hi) {
		const mid = (lo + hi) >>> 1;
		const r = ranges[mid];
		if (lineNum < r.startLine) {
			hi = mid - 1;
		} else if (lineNum > r.endLine) {
			lo = mid + 1;
		} else {
			return r;
		}
	}
	return undefined;
}

export interface ShikiDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
	diffOnlyMode?: boolean;
	language: string;
	hunks: Hunk[];
	filePath?: string;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
	comments?: DiffComment[];
	onAddComment?: (lineNumber: number, content: string) => Promise<void>;
	onAddRangeComment?: (
		startLine: number,
		endLine: number,
		content: string,
	) => Promise<void>;
	onUpdateComment?: (commentId: string, content: string) => Promise<void>;
	onDeleteComment?: (commentId: string) => Promise<void>;
	onSendComment?: (commentIds: string[]) => Promise<void>;
	scrollToLine?: number | null;
}

type VisibleItem = DiffBlock | { type: "hidden"; range: HiddenRange };

function lineBgClass(type: string): string {
	if (type === "added")
		return "bg-[color-mix(in_oklch,var(--status-added)_15%,transparent)]";
	if (type === "deleted")
		return "bg-[color-mix(in_oklch,var(--status-deleted)_15%,transparent)]";
	return "";
}

function lineMarkerClass(type: string): string {
	if (type === "added") return "text-[var(--status-added)]";
	if (type === "deleted") return "text-[var(--status-deleted)]";
	return "text-transparent";
}

function lineMarker(type: string): string {
	if (type === "added") return "+";
	if (type === "deleted") return "-";
	return " ";
}

interface HighlightRange {
	start: number;
	end: number;
	isCurrent: boolean;
}

function renderTokens(
	tokens: DiffLine["tokens"],
	highlights?: HighlightRange[],
): React.ReactNode {
	if (tokens.length === 0) return "\u00A0";

	if (!highlights || highlights.length === 0) {
		let offset = 0;
		return tokens.map((token) => {
			const key = `${offset}-${token.content.length}`;
			offset += token.content.length;
			return (
				<span key={key} style={{ color: token.color }}>
					{token.content}
				</span>
			);
		});
	}

	const result: React.ReactNode[] = [];
	let charOffset = 0;

	for (const token of tokens) {
		const tokenStart = charOffset;
		const tokenEnd = charOffset + token.content.length;
		let pos = 0;

		for (const hl of highlights) {
			if (hl.end <= tokenStart || hl.start >= tokenEnd) continue;

			const hlStartInToken = Math.max(pos, Math.max(0, hl.start - tokenStart));
			const hlEndInToken = Math.min(token.content.length, hl.end - tokenStart);

			if (hlStartInToken >= hlEndInToken) continue;

			if (hlStartInToken > pos) {
				result.push(
					<span
						key={`${charOffset}-${pos}-normal`}
						style={{ color: token.color }}
					>
						{token.content.slice(pos, hlStartInToken)}
					</span>,
				);
			}

			result.push(
				<span
					key={`${charOffset}-${hlStartInToken}-hl`}
					style={{ color: token.color }}
					className={
						hl.isCurrent
							? "bg-[var(--search-match-current,#515c6a)] outline outline-2 outline-[var(--search-match-current-border,#eccc68)]"
							: "bg-[var(--search-match,#623315)]"
					}
					data-search-match={hl.isCurrent ? "current" : "match"}
				>
					{token.content.slice(hlStartInToken, hlEndInToken)}
				</span>,
			);
			pos = hlEndInToken;
		}

		if (pos < token.content.length) {
			result.push(
				<span key={`${charOffset}-${pos}-tail`} style={{ color: token.color }}>
					{token.content.slice(pos)}
				</span>,
			);
		}

		charOffset = tokenEnd;
	}

	return result;
}

const DiffLineRow = React.memo(function DiffLineRow({
	line,
	showOldLineNumber,
	showNewLineNumber,
	commentButton,
	highlights,
}: {
	line: DiffLine;
	showOldLineNumber: boolean;
	showNewLineNumber: boolean;
	commentButton?: React.ReactNode;
	highlights?: HighlightRange[];
}) {
	return (
		<div
			className={`flex ${lineBgClass(line.type)} hover:brightness-110 min-h-[20px]`}
		>
			{showOldLineNumber && (
				<span className="shrink-0 w-[50px] text-right pr-2 text-[var(--muted-foreground)] text-xs select-none opacity-60 leading-[20px]">
					{line.oldLineNumber ?? ""}
				</span>
			)}
			{showNewLineNumber && (
				<span className="shrink-0 w-[50px] text-right pr-2 text-[var(--muted-foreground)] text-xs select-none opacity-60 leading-[20px]">
					{line.newLineNumber ?? ""}
				</span>
			)}
			<span
				className={`shrink-0 w-[20px] text-center text-xs select-none leading-[20px] font-mono ${lineMarkerClass(line.type)}`}
			>
				{lineMarker(line.type)}
			</span>
			{commentButton}
			<span className="flex-1 whitespace-pre-wrap break-all font-mono text-sm leading-[20px] pr-4 select-text">
				{renderTokens(line.tokens, highlights)}
			</span>
		</div>
	);
});

interface GutterDiffLine extends DiffLine {
	hasDeleteMarker?: boolean;
	changeGroupIndex?: number;
	isGroupStart?: boolean;
	_source?: DiffLine;
}

function buildGutterLines(blocks: DiffBlock[]): GutterDiffLine[] {
	const result: GutterDiffLine[] = [];

	for (const block of blocks) {
		if (block.type === "context") {
			for (const line of block.lines) {
				result.push({ ...line, _source: line });
			}
			continue;
		}

		let pendingDeleteMarker = false;
		const nonDeletedLines: GutterDiffLine[] = [];
		let isFirst = true;

		for (const line of block.lines) {
			if (line.type === "deleted") {
				pendingDeleteMarker = true;
			} else {
				const gutterLine: GutterDiffLine = {
					...line,
					changeGroupIndex: block.changeGroupIndex,
					_source: line,
				};
				if (pendingDeleteMarker) {
					gutterLine.hasDeleteMarker = true;
					pendingDeleteMarker = false;
				}
				if (isFirst) {
					gutterLine.isGroupStart = true;
					isFirst = false;
				}
				nonDeletedLines.push(gutterLine);
			}
		}

		if (pendingDeleteMarker && nonDeletedLines.length > 0) {
			nonDeletedLines[nonDeletedLines.length - 1].hasDeleteMarker = true;
		}

		if (nonDeletedLines.length === 0) {
			nonDeletedLines.push({
				type: "context",
				oldLineNumber: null,
				newLineNumber: null,
				tokens: [],
				content: "",
				hasDeleteMarker: true,
				changeGroupIndex: block.changeGroupIndex,
				isGroupStart: true,
			});
		}

		result.push(...nonDeletedLines);
	}

	return result;
}

const GutterLineRow = React.memo(function GutterLineRow({
	line,
	commentButton,
	highlights,
}: {
	line: GutterDiffLine;
	commentButton?: React.ReactNode;
	highlights?: HighlightRange[];
}) {
	const isAdded = line.type === "added";
	const hasDelete = line.hasDeleteMarker === true;

	const barClass = isAdded
		? "bg-[var(--status-added)]"
		: hasDelete
			? "bg-[var(--status-deleted)]"
			: "bg-transparent";

	const marker = isAdded ? "+" : hasDelete ? "-" : " ";
	const markerClass = isAdded
		? "text-[var(--status-added)]"
		: hasDelete
			? "text-[var(--status-deleted)]"
			: "text-transparent";

	return (
		<div className="flex hover:brightness-110 min-h-[20px]">
			<span className={`shrink-0 w-[4px] ml-[3px] ${barClass}`} />
			<span className="shrink-0 w-[50px] text-right pr-2 text-[var(--muted-foreground)] text-xs select-none opacity-60 leading-[20px]">
				{line.newLineNumber ?? ""}
			</span>
			<span
				className={`shrink-0 w-[20px] text-center text-xs select-none leading-[20px] font-mono ${markerClass}`}
			>
				{marker}
			</span>
			{commentButton}
			<span className="flex-1 whitespace-pre-wrap break-all font-mono text-sm leading-[20px] pr-4 select-text">
				{renderTokens(line.tokens, highlights)}
			</span>
		</div>
	);
});

function renderHalfLine(
	line: DiffLine | null,
	commentButton?: React.ReactNode,
	highlights?: HighlightRange[],
): React.ReactNode {
	if (!line) return <div className="min-h-[20px] bg-[var(--muted)]/20" />;
	return (
		<div
			className={`flex ${lineBgClass(line.type)} hover:brightness-110 min-h-[20px]`}
		>
			<span className="shrink-0 w-[50px] text-right pr-2 text-[var(--muted-foreground)] text-xs select-none opacity-60 leading-[20px]">
				{(line.type === "deleted" ? line.oldLineNumber : line.newLineNumber) ??
					""}
			</span>
			<span
				className={`shrink-0 w-[20px] text-center text-xs select-none leading-[20px] font-mono ${lineMarkerClass(line.type)}`}
			>
				{lineMarker(line.type)}
			</span>
			{commentButton}
			<span className="flex-1 whitespace-pre-wrap break-all font-mono text-sm leading-[20px] pr-4 select-text">
				{renderTokens(line.tokens, highlights)}
			</span>
		</div>
	);
}

const SplitDiffLineRow = React.memo(function SplitDiffLineRow({
	left,
	right,
	commentButton,
	leftHighlights,
	rightHighlights,
}: {
	left: DiffLine | null;
	right: DiffLine | null;
	commentButton?: React.ReactNode;
	leftHighlights?: HighlightRange[];
	rightHighlights?: HighlightRange[];
}) {
	return (
		<div className="flex">
			<div className="flex-1 border-r border-border overflow-hidden">
				{renderHalfLine(left, undefined, leftHighlights)}
			</div>
			<div className="flex-1 overflow-hidden">
				{renderHalfLine(right, commentButton, rightHighlights)}
			</div>
		</div>
	);
});

type FlatGutterItem =
	| { kind: "gutter-line"; line: GutterDiffLine }
	| { kind: "hidden"; range: HiddenRange }
	| { kind: "comment"; comment: DiffComment }
	| { kind: "comment-input"; afterLine: number };

type FlatInlineItem =
	| {
			kind: "line";
			line: DiffLine;
			showStageButton: boolean;
			changeGroupIndex?: number;
	  }
	| { kind: "hidden"; range: HiddenRange }
	| { kind: "comment"; comment: DiffComment }
	| { kind: "comment-input"; afterLine: number };

interface FlatSplitRow {
	left: DiffLine | null;
	right: DiffLine | null;
	showStageButton: boolean;
	changeGroupIndex?: number;
}

type FlatSplitItem =
	| { kind: "split-row"; row: FlatSplitRow }
	| { kind: "hidden"; range: HiddenRange }
	| { kind: "comment"; comment: DiffComment }
	| { kind: "comment-input"; afterLine: number };

interface CommentCallbacks {
	comments?: DiffComment[];
	onAddComment?: (lineNumber: number, content: string) => Promise<void>;
	onAddRangeComment?: (
		startLine: number,
		endLine: number,
		content: string,
	) => Promise<void>;
	onUpdateComment?: (commentId: string, content: string) => Promise<void>;
	onDeleteComment?: (commentId: string) => Promise<void>;
	onSendComment?: (commentIds: string[]) => Promise<void>;
}

interface VirtualViewProps extends CommentCallbacks {
	visibleBlocks: VisibleItem[];
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
	containerRef: React.RefObject<HTMLDivElement | null>;
	scrollToLine?: number | null;
	lineHighlights?: Map<DiffLine, HighlightRange[]>;
}

function flattenWithGroups(
	visibleBlocks: VisibleItem[],
	changeGroups: ChangeGroup[],
): { blocksWithGroups: DiffBlock[]; blockOrder: VisibleItem[] } {
	const diffBlocks: DiffBlock[] = [];
	for (const item of visibleBlocks) {
		if (item.type !== "hidden") {
			diffBlocks.push(item as DiffBlock);
		}
	}
	const blocksWithGroups = assignChangeGroupsToBlocks(diffBlocks, changeGroups);
	return { blocksWithGroups, blockOrder: visibleBlocks };
}

function useDelegatedClick(
	onStageGroup: ((groupIndex: number) => void) | undefined,
	onExpandRange: (range: HiddenRange) => void,
): (e: React.MouseEvent<HTMLDivElement>) => void {
	return useCallback(
		(e: React.MouseEvent<HTMLDivElement>) => {
			const target = e.target as HTMLElement;

			const stageBtn = target.closest<HTMLElement>("[data-group-index]");
			if (stageBtn && onStageGroup) {
				const idx = Number(stageBtn.dataset.groupIndex);
				if (!Number.isNaN(idx)) {
					onStageGroup(idx);
				}
				return;
			}

			const expandBtn = target.closest<HTMLElement>("[data-expand-start]");
			if (expandBtn) {
				const startLine = Number(expandBtn.dataset.expandStart);
				const endLine = Number(expandBtn.dataset.expandEnd);
				const hiddenCount = Number(expandBtn.dataset.expandCount);
				if (
					Number.isFinite(startLine) &&
					Number.isFinite(endLine) &&
					Number.isFinite(hiddenCount)
				) {
					onExpandRange({ startLine, endLine, hiddenCount });
				}
			}
		},
		[onStageGroup, onExpandRange],
	);
}

/**
 * Hook for line range selection via mousedown+drag or Shift+click.
 * On drag completion (mouseup with start !== end), immediately calls onRangeSelected.
 * On Shift+click, creates range from last clicked line to current line.
 */
function useLineRangeSelection(
	onRangeSelected: (start: number, end: number) => void,
) {
	const [selectionStart, setSelectionStart] = useState<number | null>(null);
	const [selectionEnd, setSelectionEnd] = useState<number | null>(null);
	const isDragging = useRef(false);
	const startRef = useRef<number | null>(null);
	const endRef = useRef<number | null>(null);
	const lastClickedLine = useRef<number | null>(null);
	const onRangeSelectedRef = useRef(onRangeSelected);
	onRangeSelectedRef.current = onRangeSelected;

	const selectionRange = useMemo(() => {
		if (selectionStart == null || selectionEnd == null) return null;
		const start = Math.min(selectionStart, selectionEnd);
		const end = Math.max(selectionStart, selectionEnd);
		return { start, end };
	}, [selectionStart, selectionEnd]);

	const handleLineMouseDown = useCallback(
		(lineNumber: number, shiftKey?: boolean) => {
			if (shiftKey && lastClickedLine.current != null) {
				const start = Math.min(lastClickedLine.current, lineNumber);
				const end = Math.max(lastClickedLine.current, lineNumber);
				onRangeSelectedRef.current(start, end);
				lastClickedLine.current = null;
				return;
			}
			isDragging.current = true;
			startRef.current = lineNumber;
			endRef.current = lineNumber;
			lastClickedLine.current = lineNumber;
			setSelectionStart(lineNumber);
			setSelectionEnd(lineNumber);
		},
		[],
	);

	const handleLineMouseEnter = useCallback((lineNumber: number) => {
		if (!isDragging.current) return;
		endRef.current = lineNumber;
		setSelectionEnd(lineNumber);
	}, []);

	const clearSelection = useCallback(() => {
		setSelectionStart(null);
		setSelectionEnd(null);
		isDragging.current = false;
		startRef.current = null;
		endRef.current = null;
		lastClickedLine.current = null;
	}, []);

	useEffect(() => {
		const handleUp = () => {
			if (!isDragging.current) return;
			isDragging.current = false;
			const s = startRef.current;
			const e = endRef.current;
			if (s != null && e != null && s !== e) {
				const start = Math.min(s, e);
				const end = Math.max(s, e);
				onRangeSelectedRef.current(start, end);
				lastClickedLine.current = null;
			}
			startRef.current = null;
			endRef.current = null;
			setSelectionStart(null);
			setSelectionEnd(null);
		};
		document.addEventListener("mouseup", handleUp);
		return () => document.removeEventListener("mouseup", handleUp);
	}, []);

	return {
		selectionRange,
		handleLineMouseDown,
		handleLineMouseEnter,
		clearSelection,
	};
}

function isLineInRange(
	lineNumber: number | null,
	range: { start: number; end: number } | null,
): boolean {
	if (lineNumber == null || range == null) return false;
	return lineNumber >= range.start && lineNumber <= range.end;
}

function useCommentViewState(comments: DiffComment[] | undefined) {
	const [commentInputLine, setCommentInputLine] = useState<number | null>(null);
	const [commentInputRange, setCommentInputRange] = useState<{
		start: number;
		end: number;
	} | null>(null);
	const {
		selectionRange,
		handleLineMouseDown,
		handleLineMouseEnter,
		clearSelection,
	} = useLineRangeSelection((start, end) => {
		setCommentInputRange({ start, end });
	});

	const commentHighlightLines = useMemo(() => {
		const set = new Set<number>();
		for (const c of comments ?? []) {
			if (c.lineNumber != null) {
				if (c.endLine != null) {
					for (let i = c.lineNumber; i <= c.endLine; i++) {
						set.add(i);
					}
				} else {
					set.add(c.lineNumber);
				}
			}
		}
		return set;
	}, [comments]);

	return {
		commentInputLine,
		setCommentInputLine,
		commentInputRange,
		setCommentInputRange,
		selectionRange,
		handleLineMouseDown,
		handleLineMouseEnter,
		clearSelection,
		commentHighlightLines,
	};
}

function buildCommentsByLine(
	comments: DiffComment[] | undefined,
): Map<number, DiffComment[]> {
	const map = new Map<number, DiffComment[]>();
	for (const c of comments ?? []) {
		if (c.lineNumber != null) {
			const key = c.endLine ?? c.lineNumber;
			const arr = map.get(key) ?? [];
			arr.push(c);
			map.set(key, arr);
		}
	}
	return map;
}

function insertCommentItems<T extends { kind: string }>(
	result: T[],
	lineNum: number | null,
	commentsByLine: Map<number, DiffComment[]>,
	commentInputLine: number | null,
	commentInputRange: { start: number; end: number } | null,
	makeComment: (comment: DiffComment) => T,
	makeCommentInput: (afterLine: number) => T,
) {
	if (lineNum == null) return;
	const lineComments = commentsByLine.get(lineNum);
	if (lineComments) {
		for (const c of lineComments) {
			result.push(makeComment(c));
		}
	}
	if (commentInputLine === lineNum) {
		result.push(makeCommentInput(lineNum));
	}
	if (commentInputRange && lineNum === commentInputRange.end) {
		result.push(makeCommentInput(lineNum));
	}
}

function estimateSizeWithComments(item: { kind: string }): number {
	if (item.kind === "hidden") return 22;
	if (item.kind === "comment") return 64;
	if (item.kind === "comment-input") return 120;
	return 20;
}

/**
 * Comment gutter cell: leftmost column of each diff line.
 * Shows blue "+" on row hover. Also handles mousedown/enter for drag selection.
 */
function CommentGutterCell({
	onClickSingle,
	onMouseDown,
	onMouseEnter,
}: {
	onClickSingle: () => void;
	onMouseDown: (shiftKey: boolean) => void;
	onMouseEnter: () => void;
}) {
	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: drag selection area for line range comments
		<span
			className="shrink-0 w-[20px] flex items-center justify-center cursor-pointer select-none"
			onMouseDown={(e) => {
				e.preventDefault();
				e.stopPropagation();
				onMouseDown(e.shiftKey);
			}}
			onMouseEnter={onMouseEnter}
		>
			<button
				type="button"
				className="size-[18px] rounded bg-blue-600 hover:bg-blue-500 text-white items-center justify-center hidden group-hover/line:flex"
				onClick={(e) => {
					e.stopPropagation();
					onClickSingle();
				}}
				title="Add comment"
			>
				<Plus className="size-3.5" />
			</button>
		</span>
	);
}

function HiddenBanner({ range }: { range: HiddenRange }) {
	return (
		<button
			type="button"
			className="flex w-full items-center justify-center text-xs text-muted-foreground cursor-pointer hover:bg-muted/50 border-y border-border h-[22px] bg-transparent"
			data-expand-start={range.startLine}
			data-expand-end={range.endLine}
			data-expand-count={range.hiddenCount}
		>
			··· {range.hiddenCount} lines hidden ···
		</button>
	);
}

function GroupStageButton({
	groupIndex,
	label,
}: {
	groupIndex: number;
	label: string;
}) {
	return (
		<button
			type="button"
			className="hunk-seg-btn hunk-stage absolute right-2 top-0 z-10"
			data-group-index={groupIndex}
		>
			{label}
		</button>
	);
}

function GutterView({
	visibleBlocks,
	changeGroups,
	onStageGroup,
	groupActionLabel,
	containerRef,
	lineHighlights,
	scrollToLine,
	comments,
	onAddComment,
	onAddRangeComment,
	onUpdateComment,
	onDeleteComment,
	onSendComment,
}: VirtualViewProps) {
	const {
		commentInputLine,
		setCommentInputLine,
		commentInputRange,
		setCommentInputRange,
		selectionRange,
		handleLineMouseDown,
		handleLineMouseEnter,
		clearSelection,
		commentHighlightLines,
	} = useCommentViewState(comments);

	const flatItems = useMemo(() => {
		const { blocksWithGroups, blockOrder } = flattenWithGroups(
			visibleBlocks,
			changeGroups ?? [],
		);
		const result: FlatGutterItem[] = [];
		let blockIdx = 0;
		const commentsByLine = buildCommentsByLine(comments);

		for (const item of blockOrder) {
			if (item.type === "hidden") {
				result.push({ kind: "hidden", range: item.range });
			} else {
				const block = blocksWithGroups[blockIdx++];
				const gutterLines = buildGutterLines([block]);
				for (const line of gutterLines) {
					result.push({ kind: "gutter-line", line });
					insertCommentItems(
						result,
						line.newLineNumber,
						commentsByLine,
						commentInputLine,
						commentInputRange,
						(c) => ({ kind: "comment" as const, comment: c }),
						(afterLine) => ({ kind: "comment-input" as const, afterLine }),
					);
				}
			}
		}

		return result;
	}, [
		visibleBlocks,
		changeGroups,
		comments,
		commentInputLine,
		commentInputRange,
	]);

	const virtualizer = useVirtualizer({
		count: flatItems.length,
		getScrollElement: () => containerRef.current,
		estimateSize: (i) => estimateSizeWithComments(flatItems[i]),
		overscan: 15,
	});

	useEffect(() => {
		if (scrollToLine == null) return;
		const index = flatItems.findIndex(
			(item) =>
				item.kind === "gutter-line" && item.line.newLineNumber === scrollToLine,
		);
		if (index >= 0) {
			virtualizer.scrollToIndex(index, { align: "center" });
		}
	}, [scrollToLine, flatItems, virtualizer]);

	return (
		<div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
			{virtualizer.getVirtualItems().map((vItem) => {
				const item = flatItems[vItem.index];
				return (
					<div
						key={vItem.index}
						data-index={vItem.index}
						ref={virtualizer.measureElement}
						style={{
							position: "absolute",
							top: 0,
							left: 0,
							width: "100%",
							transform: `translateY(${vItem.start}px)`,
						}}
					>
						{item.kind === "hidden" ? (
							<HiddenBanner range={item.range} />
						) : item.kind === "comment" ? (
							<DiffInlineComment
								comment={item.comment}
								onUpdate={onUpdateComment ?? (async () => {})}
								onDelete={onDeleteComment ?? (async () => {})}
								onSend={onSendComment ?? (async () => {})}
							/>
						) : item.kind === "comment-input" ? (
							<DiffInlineCommentInput
								onSubmit={async (content) => {
									if (commentInputRange) {
										if (onAddRangeComment) {
											await onAddRangeComment(
												commentInputRange.start,
												commentInputRange.end,
												content,
											);
										} else {
											await onAddComment?.(commentInputRange.start, content);
										}
										setCommentInputRange(null);
									} else {
										await onAddComment?.(item.afterLine, content);
										setCommentInputLine(null);
									}
									clearSelection();
								}}
								onCancel={() => {
									setCommentInputLine(null);
									setCommentInputRange(null);
									clearSelection();
								}}
								rangeLabel={
									commentInputRange
										? `L${commentInputRange.start}-${commentInputRange.end}`
										: undefined
								}
							/>
						) : (
							// biome-ignore lint/a11y/noStaticElementInteractions: drag range tracking
							<div
								data-diff-line={item.line.newLineNumber ?? undefined}
								className={`group/line relative ${isLineInRange(item.line.newLineNumber, selectionRange) || item.line.newLineNumber === commentInputLine || isLineInRange(item.line.newLineNumber, commentInputRange) ? "bg-[color-mix(in_oklch,var(--color-blue-500)_15%,transparent)]" : commentHighlightLines.has(item.line.newLineNumber ?? -1) ? "bg-[color-mix(in_oklch,var(--color-blue-500)_8%,transparent)]" : ""}`}
								onMouseEnter={() => {
									if (item.line.newLineNumber != null)
										handleLineMouseEnter(item.line.newLineNumber);
								}}
								onMouseDown={(e) => {
									const target = e.target as HTMLElement;
									if (
										target.closest(".select-none") &&
										item.line.newLineNumber != null
									) {
										e.preventDefault();
										handleLineMouseDown(item.line.newLineNumber, e.shiftKey);
									}
								}}
							>
								{item.line.isGroupStart &&
									item.line.changeGroupIndex != null &&
									onStageGroup && (
										<GroupStageButton
											groupIndex={item.line.changeGroupIndex}
											label={groupActionLabel ?? "Stage"}
										/>
									)}
								<GutterLineRow
									line={item.line}
									highlights={lineHighlights?.get(
										item.line._source ?? item.line,
									)}
									commentButton={
										onAddComment ? (
											<CommentGutterCell
												onClickSingle={() => {
													setCommentInputLine(item.line.newLineNumber);
													clearSelection();
												}}
												onMouseDown={(shiftKey) => {
													if (item.line.newLineNumber != null)
														handleLineMouseDown(
															item.line.newLineNumber,
															shiftKey,
														);
												}}
												onMouseEnter={() => {
													if (item.line.newLineNumber != null)
														handleLineMouseEnter(item.line.newLineNumber);
												}}
											/>
										) : undefined
									}
								/>
							</div>
						)}
					</div>
				);
			})}
		</div>
	);
}

function InlineView({
	visibleBlocks,
	changeGroups,
	onStageGroup,
	groupActionLabel,
	containerRef,
	lineHighlights,
	scrollToLine,
	comments,
	onAddComment,
	onAddRangeComment,
	onUpdateComment,
	onDeleteComment,
	onSendComment,
}: VirtualViewProps) {
	const {
		commentInputLine,
		setCommentInputLine,
		commentInputRange,
		setCommentInputRange,
		selectionRange,
		handleLineMouseDown,
		handleLineMouseEnter,
		clearSelection,
		commentHighlightLines,
	} = useCommentViewState(comments);

	const flatItems = useMemo(() => {
		const { blocksWithGroups, blockOrder } = flattenWithGroups(
			visibleBlocks,
			changeGroups ?? [],
		);
		const result: FlatInlineItem[] = [];
		let blockIdx = 0;
		const commentsByLine = buildCommentsByLine(comments);

		for (const item of blockOrder) {
			if (item.type === "hidden") {
				result.push({ kind: "hidden", range: item.range });
			} else {
				const block = blocksWithGroups[blockIdx++];
				let isFirst = true;
				for (const line of block.lines) {
					result.push({
						kind: "line",
						line,
						showStageButton: isFirst && block.changeGroupIndex != null,
						changeGroupIndex: block.changeGroupIndex,
					});
					isFirst = false;

					insertCommentItems(
						result,
						line.newLineNumber,
						commentsByLine,
						commentInputLine,
						commentInputRange,
						(c) => ({ kind: "comment" as const, comment: c }),
						(afterLine) => ({ kind: "comment-input" as const, afterLine }),
					);
				}
			}
		}

		return result;
	}, [
		visibleBlocks,
		changeGroups,
		comments,
		commentInputLine,
		commentInputRange,
	]);

	const virtualizer = useVirtualizer({
		count: flatItems.length,
		getScrollElement: () => containerRef.current,
		estimateSize: (i) => estimateSizeWithComments(flatItems[i]),
		overscan: 15,
	});

	useEffect(() => {
		if (scrollToLine == null) return;
		const index = flatItems.findIndex(
			(item) =>
				item.kind === "line" && item.line.newLineNumber === scrollToLine,
		);
		if (index >= 0) {
			virtualizer.scrollToIndex(index, { align: "center" });
		}
	}, [scrollToLine, flatItems, virtualizer]);

	return (
		<div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
			{virtualizer.getVirtualItems().map((vItem) => {
				const item = flatItems[vItem.index];
				return (
					<div
						key={vItem.index}
						data-index={vItem.index}
						ref={virtualizer.measureElement}
						style={{
							position: "absolute",
							top: 0,
							left: 0,
							width: "100%",
							transform: `translateY(${vItem.start}px)`,
						}}
					>
						{item.kind === "hidden" ? (
							<HiddenBanner range={item.range} />
						) : item.kind === "comment" ? (
							<DiffInlineComment
								comment={item.comment}
								onUpdate={onUpdateComment ?? (async () => {})}
								onDelete={onDeleteComment ?? (async () => {})}
								onSend={onSendComment ?? (async () => {})}
							/>
						) : item.kind === "comment-input" ? (
							<DiffInlineCommentInput
								onSubmit={async (content) => {
									if (commentInputRange) {
										if (onAddRangeComment) {
											await onAddRangeComment(
												commentInputRange.start,
												commentInputRange.end,
												content,
											);
										} else {
											await onAddComment?.(commentInputRange.start, content);
										}
										setCommentInputRange(null);
									} else {
										await onAddComment?.(item.afterLine, content);
										setCommentInputLine(null);
									}
									clearSelection();
								}}
								onCancel={() => {
									setCommentInputLine(null);
									setCommentInputRange(null);
									clearSelection();
								}}
								rangeLabel={
									commentInputRange
										? `L${commentInputRange.start}-${commentInputRange.end}`
										: undefined
								}
							/>
						) : (
							// biome-ignore lint/a11y/noStaticElementInteractions: drag range tracking
							<div
								data-diff-line={item.line.newLineNumber ?? undefined}
								className={`group/line relative ${isLineInRange(item.line.newLineNumber, selectionRange) || item.line.newLineNumber === commentInputLine || isLineInRange(item.line.newLineNumber, commentInputRange) ? "bg-[color-mix(in_oklch,var(--color-blue-500)_15%,transparent)]" : commentHighlightLines.has(item.line.newLineNumber ?? -1) ? "bg-[color-mix(in_oklch,var(--color-blue-500)_8%,transparent)]" : ""}`}
								onMouseEnter={() => {
									if (item.line.newLineNumber != null)
										handleLineMouseEnter(item.line.newLineNumber);
								}}
								onMouseDown={(e) => {
									const target = e.target as HTMLElement;
									if (
										target.closest(".select-none") &&
										item.line.newLineNumber != null
									) {
										e.preventDefault();
										handleLineMouseDown(item.line.newLineNumber, e.shiftKey);
									}
								}}
							>
								{item.showStageButton &&
									item.changeGroupIndex != null &&
									onStageGroup && (
										<GroupStageButton
											groupIndex={item.changeGroupIndex}
											label={groupActionLabel ?? "Stage"}
										/>
									)}
								<DiffLineRow
									line={item.line}
									showOldLineNumber={true}
									showNewLineNumber={true}
									highlights={lineHighlights?.get(item.line)}
									commentButton={
										onAddComment ? (
											<CommentGutterCell
												onClickSingle={() => {
													setCommentInputLine(item.line.newLineNumber);
													clearSelection();
												}}
												onMouseDown={(shiftKey) => {
													if (item.line.newLineNumber != null)
														handleLineMouseDown(
															item.line.newLineNumber,
															shiftKey,
														);
												}}
												onMouseEnter={() => {
													if (item.line.newLineNumber != null)
														handleLineMouseEnter(item.line.newLineNumber);
												}}
											/>
										) : undefined
									}
								/>
							</div>
						)}
					</div>
				);
			})}
		</div>
	);
}

function SplitView({
	visibleBlocks,
	changeGroups,
	onStageGroup,
	groupActionLabel,
	containerRef,
	lineHighlights,
	scrollToLine,
	comments,
	onAddComment,
	onAddRangeComment,
	onUpdateComment,
	onDeleteComment,
	onSendComment,
}: VirtualViewProps) {
	const {
		commentInputLine,
		setCommentInputLine,
		commentInputRange,
		setCommentInputRange,
		selectionRange,
		handleLineMouseDown,
		handleLineMouseEnter,
		clearSelection,
		commentHighlightLines,
	} = useCommentViewState(comments);

	const flatItems = useMemo(() => {
		const { blocksWithGroups, blockOrder } = flattenWithGroups(
			visibleBlocks,
			changeGroups ?? [],
		);
		const result: FlatSplitItem[] = [];
		let blockIdx = 0;
		const commentsByLine = buildCommentsByLine(comments);

		for (const item of blockOrder) {
			if (item.type === "hidden") {
				result.push({ kind: "hidden", range: item.range });
			} else {
				const block = blocksWithGroups[blockIdx++];

				if (block.type === "context") {
					for (const line of block.lines) {
						result.push({
							kind: "split-row",
							row: {
								left: line,
								right: line,
								showStageButton: false,
							},
						});
						insertCommentItems(
							result,
							line.newLineNumber,
							commentsByLine,
							commentInputLine,
							commentInputRange,
							(c) => ({ kind: "comment" as const, comment: c }),
							(afterLine) => ({ kind: "comment-input" as const, afterLine }),
						);
					}
				} else {
					const deleted = block.lines.filter((l) => l.type === "deleted");
					const added = block.lines.filter((l) => l.type === "added");
					const contextInBlock = block.lines.filter(
						(l) => l.type === "context",
					);

					const maxLen = Math.max(deleted.length, added.length);
					for (let i = 0; i < maxLen; i++) {
						result.push({
							kind: "split-row",
							row: {
								left: deleted[i] ?? null,
								right: added[i] ?? null,
								showStageButton: i === 0 && block.changeGroupIndex != null,
								changeGroupIndex: block.changeGroupIndex,
							},
						});
						const rightLine = added[i];
						insertCommentItems(
							result,
							rightLine?.newLineNumber ?? null,
							commentsByLine,
							commentInputLine,
							commentInputRange,
							(c) => ({ kind: "comment" as const, comment: c }),
							(afterLine) => ({ kind: "comment-input" as const, afterLine }),
						);
					}

					for (const line of contextInBlock) {
						result.push({
							kind: "split-row",
							row: {
								left: line,
								right: line,
								showStageButton: false,
							},
						});
					}
				}
			}
		}

		return result;
	}, [
		visibleBlocks,
		changeGroups,
		comments,
		commentInputLine,
		commentInputRange,
	]);

	const virtualizer = useVirtualizer({
		count: flatItems.length,
		getScrollElement: () => containerRef.current,
		estimateSize: (i) => estimateSizeWithComments(flatItems[i]),
		overscan: 15,
	});

	useEffect(() => {
		if (scrollToLine == null) return;
		const index = flatItems.findIndex(
			(item) =>
				item.kind === "split-row" &&
				item.row.right?.newLineNumber === scrollToLine,
		);
		if (index >= 0) {
			virtualizer.scrollToIndex(index, { align: "center" });
		}
	}, [scrollToLine, flatItems, virtualizer]);

	return (
		<div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
			{virtualizer.getVirtualItems().map((vItem) => {
				const item = flatItems[vItem.index];
				return (
					<div
						key={vItem.index}
						data-index={vItem.index}
						ref={virtualizer.measureElement}
						style={{
							position: "absolute",
							top: 0,
							left: 0,
							width: "100%",
							transform: `translateY(${vItem.start}px)`,
						}}
					>
						{item.kind === "hidden" ? (
							<HiddenBanner range={item.range} />
						) : item.kind === "comment" ? (
							<DiffInlineComment
								comment={item.comment}
								onUpdate={onUpdateComment ?? (async () => {})}
								onDelete={onDeleteComment ?? (async () => {})}
								onSend={onSendComment ?? (async () => {})}
							/>
						) : item.kind === "comment-input" ? (
							<DiffInlineCommentInput
								onSubmit={async (content) => {
									if (commentInputRange) {
										if (onAddRangeComment) {
											await onAddRangeComment(
												commentInputRange.start,
												commentInputRange.end,
												content,
											);
										} else {
											await onAddComment?.(commentInputRange.start, content);
										}
										setCommentInputRange(null);
									} else {
										await onAddComment?.(item.afterLine, content);
										setCommentInputLine(null);
									}
									clearSelection();
								}}
								onCancel={() => {
									setCommentInputLine(null);
									setCommentInputRange(null);
									clearSelection();
								}}
								rangeLabel={
									commentInputRange
										? `L${commentInputRange.start}-${commentInputRange.end}`
										: undefined
								}
							/>
						) : (
							// biome-ignore lint/a11y/noStaticElementInteractions: drag range tracking
							<div
								data-diff-line={item.row.right?.newLineNumber ?? undefined}
								className={`group/line relative ${isLineInRange(item.row.right?.newLineNumber ?? null, selectionRange) || (item.row.right?.newLineNumber ?? null) === commentInputLine || isLineInRange(item.row.right?.newLineNumber ?? null, commentInputRange) ? "bg-[color-mix(in_oklch,var(--color-blue-500)_15%,transparent)]" : commentHighlightLines.has(item.row.right?.newLineNumber ?? -1) ? "bg-[color-mix(in_oklch,var(--color-blue-500)_8%,transparent)]" : ""}`}
								onMouseEnter={() => {
									const lineNum = item.row.right?.newLineNumber;
									if (lineNum != null) handleLineMouseEnter(lineNum);
								}}
								onMouseDown={(e) => {
									const target = e.target as HTMLElement;
									const lineNum = item.row.right?.newLineNumber;
									if (target.closest(".select-none") && lineNum != null) {
										e.preventDefault();
										handleLineMouseDown(lineNum, e.shiftKey);
									}
								}}
							>
								{item.row.showStageButton &&
									item.row.changeGroupIndex != null &&
									onStageGroup && (
										<GroupStageButton
											groupIndex={item.row.changeGroupIndex}
											label={groupActionLabel ?? "Stage"}
										/>
									)}
								<SplitDiffLineRow
									left={item.row.left}
									right={item.row.right}
									leftHighlights={
										item.row.left
											? lineHighlights?.get(item.row.left)
											: undefined
									}
									rightHighlights={
										item.row.right
											? lineHighlights?.get(item.row.right)
											: undefined
									}
									commentButton={
										onAddComment ? (
											<CommentGutterCell
												onClickSingle={() => {
													setCommentInputLine(
														item.row.right?.newLineNumber ?? null,
													);
													clearSelection();
												}}
												onMouseDown={(shiftKey) => {
													const lineNum = item.row.right?.newLineNumber;
													if (lineNum != null)
														handleLineMouseDown(lineNum, shiftKey);
												}}
												onMouseEnter={() => {
													const lineNum = item.row.right?.newLineNumber;
													if (lineNum != null) handleLineMouseEnter(lineNum);
												}}
											/>
										) : undefined
									}
								/>
							</div>
						)}
					</div>
				);
			})}
		</div>
	);
}

interface DiffMarker {
	position: number;
	height: number;
	type: "added" | "deleted";
}

function computeBlockLineCount(block: DiffBlock, mode: DiffMode): number {
	if (block.type === "context") return block.lines.length;
	if (mode === "gutter") return buildGutterLines([block]).length;
	if (mode === "split") {
		const deleted = block.lines.filter((l) => l.type === "deleted").length;
		const added = block.lines.filter((l) => l.type === "added").length;
		const ctx = block.lines.filter((l) => l.type === "context").length;
		return Math.max(deleted, added) + ctx;
	}
	return block.lines.length;
}

function flushRun(
	markers: DiffMarker[],
	runStart: number,
	runLen: number,
	totalLines: number,
	runType: "added" | "deleted",
): void {
	if (runLen <= 0) return;
	markers.push({
		position: runStart / totalLines,
		height: runLen / totalLines,
		type: runType,
	});
}

function computeDiffMarkers(
	visibleBlocks: VisibleItem[],
	diffMode: DiffMode,
): DiffMarker[] {
	let totalLines = 0;
	for (const item of visibleBlocks) {
		if (item.type === "hidden") {
			totalLines += 1;
		} else {
			totalLines += computeBlockLineCount(item as DiffBlock, diffMode);
		}
	}
	if (totalLines === 0) return [];

	const markers: DiffMarker[] = [];
	let currentLine = 0;

	for (const item of visibleBlocks) {
		if (item.type === "hidden") {
			currentLine += 1;
			continue;
		}
		const block = item as DiffBlock;

		if (block.type === "change") {
			let runStart = currentLine;
			let runLen = 0;
			let runType: "added" | "deleted" | null = null;

			for (const line of block.lines) {
				if (line.type === "context") {
					flushRun(markers, runStart, runLen, totalLines, runType ?? "added");
					runLen = 0;
					runType = null;
					currentLine += 1;
					runStart = currentLine;
				} else if (diffMode === "gutter" && line.type === "deleted") {
					// Gutter mode: deleted lines are collapsed, skip
				} else {
					if (runType !== null && runType !== line.type) {
						flushRun(markers, runStart, runLen, totalLines, runType);
						runLen = 0;
						runStart = currentLine;
					}
					runType = line.type as "added" | "deleted";
					runLen += 1;
					currentLine += 1;
				}
			}

			if (runLen > 0 && runType !== null) {
				flushRun(markers, runStart, runLen, totalLines, runType);
			}
		} else {
			currentLine += block.lines.length;
		}
	}

	return markers;
}

const MARKER_COLORS: Record<DiffMarker["type"], string> = {
	added: "color-mix(in oklch, var(--status-added) 55%, transparent)",
	deleted: "color-mix(in oklch, var(--status-deleted) 55%, transparent)",
};

const ScrollbarMarkers = React.memo(function ScrollbarMarkers({
	markers,
}: {
	markers: DiffMarker[];
}) {
	if (markers.length === 0) return null;

	return (
		<div
			className="absolute top-0 right-0 h-full pointer-events-none"
			style={{ width: "10px" }}
			aria-hidden="true"
			data-testid="scrollbar-markers"
		>
			{markers.map((marker) => {
				const key = `${marker.position}-${marker.type}`;
				return (
					<div
						key={key}
						className="absolute"
						style={{
							top: `${marker.position * 100}%`,
							height: `${Math.max(marker.height * 100, 0.4)}%`,
							minHeight: "2px",
							width: "8px",
							right: "1px",
							backgroundColor: MARKER_COLORS[marker.type],
						}}
					/>
				);
			})}
		</div>
	);
});

function collectAllLines(visibleBlocks: VisibleItem[]): DiffLine[] {
	const result: DiffLine[] = [];
	for (const item of visibleBlocks) {
		if (item.type !== "hidden") {
			for (const line of (item as DiffBlock).lines) {
				result.push(line);
			}
		}
	}
	return result;
}

function buildLineHighlights(
	allLines: DiffLine[],
	matches: SearchMatch[],
	currentIndex: number,
): Map<DiffLine, HighlightRange[]> {
	const map = new Map<DiffLine, HighlightRange[]>();
	for (let i = 0; i < matches.length; i++) {
		const m = matches[i];
		const line = allLines[m.lineIndex];
		if (!line) continue;
		let ranges = map.get(line);
		if (!ranges) {
			ranges = [];
			map.set(line, ranges);
		}
		ranges.push({
			start: m.startOffset,
			end: m.endOffset,
			isCurrent: i === currentIndex,
		});
	}
	return map;
}

export function ShikiDiffViewer({
	originalContent,
	modifiedContent,
	diffMode,
	diffOnlyMode,
	language,
	hunks,
	filePath,
	changeGroups,
	onStageGroup,
	groupActionLabel,
	comments,
	onAddComment,
	onAddRangeComment,
	onUpdateComment,
	onDeleteComment,
	onSendComment,
	scrollToLine,
}: ShikiDiffViewerProps) {
	const originalTokens = useShikiHighlighter(originalContent, language);
	const modifiedTokens = useShikiHighlighter(modifiedContent, language);

	const { blocks } = useMemo(
		() =>
			computeDiffBlocks(
				hunks,
				originalTokens,
				modifiedTokens,
				originalContent,
				modifiedContent,
			),
		[hunks, originalTokens, modifiedTokens, originalContent, modifiedContent],
	);

	const [hiddenRanges, setHiddenRanges] = useState<HiddenRange[]>([]);

	useEffect(() => {
		if (!diffOnlyMode) {
			setHiddenRanges([]);
			return;
		}

		let cancelled = false;
		invoke<HiddenRange[]>("compute_hidden_ranges_from_content", {
			original: originalContent,
			modified: modifiedContent,
			contextLines: 3,
		})
			.then((ranges) => {
				if (!cancelled) setHiddenRanges(ranges);
			})
			.catch(() => {
				if (!cancelled) setHiddenRanges([]);
			});

		return () => {
			cancelled = true;
		};
	}, [diffOnlyMode, originalContent, modifiedContent]);

	const expandRange = useCallback((range: HiddenRange) => {
		setHiddenRanges((prev) =>
			prev.filter(
				(r) => r.startLine !== range.startLine || r.endLine !== range.endLine,
			),
		);
	}, []);

	const visibleBlocks = useMemo(() => {
		if (!diffOnlyMode || hiddenRanges.length === 0) return blocks;

		const result: VisibleItem[] = [];

		for (const block of blocks) {
			if (block.type === "change") {
				result.push(block);
				continue;
			}

			const visibleLines: DiffLine[] = [];
			let currentHiddenRange: HiddenRange | null = null;

			for (const line of block.lines) {
				const lineNum = line.newLineNumber ?? line.oldLineNumber ?? 0;
				const hidden = findHiddenRange(lineNum, hiddenRanges);

				if (hidden) {
					if (visibleLines.length > 0) {
						result.push({ type: "context", lines: [...visibleLines] });
						visibleLines.length = 0;
					}
					if (
						!currentHiddenRange ||
						currentHiddenRange.startLine !== hidden.startLine
					) {
						currentHiddenRange = hidden;
						result.push({ type: "hidden", range: hidden });
					}
				} else {
					currentHiddenRange = null;
					visibleLines.push(line);
				}
			}

			if (visibleLines.length > 0) {
				result.push({ type: "context", lines: visibleLines });
			}
		}

		return result;
	}, [blocks, diffOnlyMode, hiddenRanges]);

	const containerRef = useRef<HTMLDivElement>(null);

	const prevFilePathRef = useRef(filePath);
	useEffect(() => {
		if (prevFilePathRef.current !== filePath) {
			prevFilePathRef.current = filePath;
			if (
				containerRef.current &&
				typeof containerRef.current.scrollTo === "function"
			) {
				containerRef.current.scrollTo(0, 0);
			}
		}
	}, [filePath]);

	const handleDelegatedClick = useDelegatedClick(onStageGroup, expandRange);

	const diffMarkers = useMemo(
		() => computeDiffMarkers(visibleBlocks, diffMode),
		[visibleBlocks, diffMode],
	);

	const allLines = useMemo(
		() => collectAllLines(visibleBlocks),
		[visibleBlocks],
	);

	const search = useDiffSearch(allLines);

	const lineHighlights = useMemo(
		() => buildLineHighlights(allLines, search.matches, search.currentIndex),
		[allLines, search.matches, search.currentIndex],
	);

	const wrapperRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		const el = wrapperRef.current;
		if (!el) return;

		const handleKeyDown = (e: KeyboardEvent) => {
			if ((e.metaKey || e.ctrlKey) && e.key === "f") {
				e.preventDefault();
				search.open();
			}
		};

		el.addEventListener("keydown", handleKeyDown);
		return () => el.removeEventListener("keydown", handleKeyDown);
	}, [search.open]);

	const searchScrollToLine = useMemo(() => {
		if (search.matches.length === 0 || search.currentIndex < 0) return null;
		const match = search.matches[search.currentIndex];
		const line = allLines[match.lineIndex];
		return line?.newLineNumber ?? line?.oldLineNumber ?? null;
	}, [search.matches, search.currentIndex, allLines]);

	const ViewComponent =
		diffMode === "gutter"
			? GutterView
			: diffMode === "split"
				? SplitView
				: InlineView;

	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: event delegation for stage buttons and expand banners
		// biome-ignore lint/a11y/useKeyWithClickEvents: interactive elements inside handle keyboard events
		<div
			ref={wrapperRef}
			className="relative h-full w-full"
			onClick={handleDelegatedClick}
			tabIndex={-1}
		>
			{search.isOpen && (
				<DiffSearchBar
					query={search.query}
					onQueryChange={search.setQuery}
					currentIndex={search.currentIndex}
					totalMatches={search.totalMatches}
					onNext={search.goToNext}
					onPrev={search.goToPrev}
					onClose={search.close}
				/>
			)}
			<ScrollArea
				viewportRef={containerRef}
				className="h-full w-full font-mono text-sm"
				style={{
					backgroundColor: "var(--editor-background, #1a1a1a)",
					color: "var(--editor-foreground, #e0e0e0)",
				}}
				data-testid="code-diff-viewer"
			>
				<ViewComponent
					visibleBlocks={visibleBlocks}
					changeGroups={changeGroups}
					onStageGroup={onStageGroup}
					groupActionLabel={groupActionLabel}
					containerRef={containerRef}
					lineHighlights={lineHighlights}
					scrollToLine={searchScrollToLine ?? scrollToLine}
					comments={comments}
					onAddComment={onAddComment}
					onAddRangeComment={onAddRangeComment}
					onUpdateComment={onUpdateComment}
					onDeleteComment={onDeleteComment}
					onSendComment={onSendComment}
				/>
				<ScrollBar orientation="horizontal" />
			</ScrollArea>
			<ScrollbarMarkers markers={diffMarkers} />
		</div>
	);
}
