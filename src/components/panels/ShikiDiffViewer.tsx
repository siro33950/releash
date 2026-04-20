import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke } from "@tauri-apps/api/core";
import React, {
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	assignChangeGroupsToBlocks,
	computeDiffBlocks,
	type DiffBlock,
	type DiffLine,
} from "@/hooks/useDiffTokens";
import { useShikiHighlighter } from "@/hooks/useShikiHighlighter";
import type { ChangeGroup, Hunk } from "@/lib/computeHunks";
import type { DiffMode } from "@/types/settings";

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

function renderTokens(tokens: DiffLine["tokens"]): React.ReactNode {
	if (tokens.length === 0) return "\u00A0";
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

const DiffLineRow = React.memo(function DiffLineRow({
	line,
	showOldLineNumber,
	showNewLineNumber,
}: {
	line: DiffLine;
	showOldLineNumber: boolean;
	showNewLineNumber: boolean;
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
			<span className="flex-1 whitespace-pre-wrap break-all font-mono text-sm leading-[20px] pr-4">
				{renderTokens(line.tokens)}
			</span>
		</div>
	);
});

interface GutterDiffLine extends DiffLine {
	hasDeleteMarker?: boolean;
	changeGroupIndex?: number;
	isGroupStart?: boolean;
}

function buildGutterLines(blocks: DiffBlock[]): GutterDiffLine[] {
	const result: GutterDiffLine[] = [];

	for (const block of blocks) {
		if (block.type === "context") {
			for (const line of block.lines) {
				result.push({ ...line });
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
}: {
	line: GutterDiffLine;
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
			<span className="flex-1 whitespace-pre-wrap break-all font-mono text-sm leading-[20px] pr-4">
				{renderTokens(line.tokens)}
			</span>
		</div>
	);
});

function renderHalfLine(line: DiffLine | null): React.ReactNode {
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
			<span className="flex-1 whitespace-pre-wrap break-all font-mono text-sm leading-[20px] pr-4">
				{renderTokens(line.tokens)}
			</span>
		</div>
	);
}

const SplitDiffLineRow = React.memo(function SplitDiffLineRow({
	left,
	right,
}: {
	left: DiffLine | null;
	right: DiffLine | null;
}) {
	return (
		<div className="flex">
			<div className="flex-1 border-r border-border overflow-hidden">
				{renderHalfLine(left)}
			</div>
			<div className="flex-1 overflow-hidden">{renderHalfLine(right)}</div>
		</div>
	);
});

type FlatGutterItem =
	| { kind: "gutter-line"; line: GutterDiffLine }
	| { kind: "hidden"; range: HiddenRange };

type FlatInlineItem =
	| {
			kind: "line";
			line: DiffLine;
			showStageButton: boolean;
			changeGroupIndex?: number;
	  }
	| { kind: "hidden"; range: HiddenRange };

interface FlatSplitRow {
	left: DiffLine | null;
	right: DiffLine | null;
	showStageButton: boolean;
	changeGroupIndex?: number;
}

type FlatSplitItem =
	| { kind: "split-row"; row: FlatSplitRow }
	| { kind: "hidden"; range: HiddenRange };

interface VirtualViewProps {
	visibleBlocks: VisibleItem[];
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
	containerRef: React.RefObject<HTMLDivElement | null>;
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
				if (!Number.isNaN(startLine)) {
					onExpandRange({ startLine, endLine, hiddenCount });
				}
			}
		},
		[onStageGroup, onExpandRange],
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
}: VirtualViewProps) {
	const flatItems = useMemo(() => {
		const { blocksWithGroups, blockOrder } = flattenWithGroups(
			visibleBlocks,
			changeGroups ?? [],
		);
		const result: FlatGutterItem[] = [];
		let blockIdx = 0;

		for (const item of blockOrder) {
			if (item.type === "hidden") {
				result.push({ kind: "hidden", range: item.range });
			} else {
				const block = blocksWithGroups[blockIdx++];
				const gutterLines = buildGutterLines([block]);
				for (const line of gutterLines) {
					result.push({ kind: "gutter-line", line });
				}
			}
		}

		return result;
	}, [visibleBlocks, changeGroups]);

	const virtualizer = useVirtualizer({
		count: flatItems.length,
		getScrollElement: () => containerRef.current,
		estimateSize: (i) => (flatItems[i].kind === "hidden" ? 22 : 20),
		overscan: 15,
	});

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
						) : (
							<>
								{item.line.isGroupStart &&
									item.line.changeGroupIndex != null &&
									onStageGroup && (
										<GroupStageButton
											groupIndex={item.line.changeGroupIndex}
											label={groupActionLabel ?? "Stage"}
										/>
									)}
								<GutterLineRow line={item.line} />
							</>
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
}: VirtualViewProps) {
	const flatItems = useMemo(() => {
		const { blocksWithGroups, blockOrder } = flattenWithGroups(
			visibleBlocks,
			changeGroups ?? [],
		);
		const result: FlatInlineItem[] = [];
		let blockIdx = 0;

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
				}
			}
		}

		return result;
	}, [visibleBlocks, changeGroups]);

	const virtualizer = useVirtualizer({
		count: flatItems.length,
		getScrollElement: () => containerRef.current,
		estimateSize: (i) => (flatItems[i].kind === "hidden" ? 22 : 20),
		overscan: 15,
	});

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
						) : (
							<>
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
								/>
							</>
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
}: VirtualViewProps) {
	const flatItems = useMemo(() => {
		const { blocksWithGroups, blockOrder } = flattenWithGroups(
			visibleBlocks,
			changeGroups ?? [],
		);
		const result: FlatSplitItem[] = [];
		let blockIdx = 0;

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
	}, [visibleBlocks, changeGroups]);

	const virtualizer = useVirtualizer({
		count: flatItems.length,
		getScrollElement: () => containerRef.current,
		estimateSize: (i) => (flatItems[i].kind === "hidden" ? 22 : 20),
		overscan: 15,
	});

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
						) : (
							<>
								{item.row.showStageButton &&
									item.row.changeGroupIndex != null &&
									onStageGroup && (
										<GroupStageButton
											groupIndex={item.row.changeGroupIndex}
											label={groupActionLabel ?? "Stage"}
										/>
									)}
								<SplitDiffLineRow left={item.row.left} right={item.row.right} />
							</>
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
	});

	const handleDelegatedClick = useDelegatedClick(onStageGroup, expandRange);

	const diffMarkers = useMemo(
		() => computeDiffMarkers(visibleBlocks, diffMode),
		[visibleBlocks, diffMode],
	);

	const ViewComponent =
		diffMode === "gutter"
			? GutterView
			: diffMode === "split"
				? SplitView
				: InlineView;

	return (
		<div className="relative h-full w-full">
			{/* biome-ignore lint/a11y/noStaticElementInteractions: event delegation for stage buttons and expand banners */}
			{/* biome-ignore lint/a11y/useKeyWithClickEvents: interactive elements inside handle keyboard events */}
			<div
				ref={containerRef}
				className="h-full w-full overflow-auto font-mono text-sm"
				style={{
					backgroundColor: "var(--editor-background, #1a1a1a)",
					color: "var(--editor-foreground, #e0e0e0)",
				}}
				data-testid="code-diff-viewer"
				onClick={handleDelegatedClick}
			>
				<ViewComponent
					visibleBlocks={visibleBlocks}
					changeGroups={changeGroups}
					onStageGroup={onStageGroup}
					groupActionLabel={groupActionLabel}
					containerRef={containerRef}
				/>
			</div>
			<ScrollbarMarkers markers={diffMarkers} />
		</div>
	);
}
