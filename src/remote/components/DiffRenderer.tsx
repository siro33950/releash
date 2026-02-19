import { useCallback, useMemo, useRef } from "react";
import {
	type ChangeGroup,
	computeChangeGroups,
	computeHunks,
	type Hunk,
} from "@/lib/computeHunks";

interface LineRange {
	start: number;
	end: number;
}

interface DiffRendererProps {
	original: string;
	modified: string;
	filePath: string;
	selectionStart: number | null;
	highlightRange: LineRange | null;
	onLineTap: (lineNumber: number) => void;
	onLineLongPress: (lineNumber: number) => void;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	onUnstageGroup?: (groupIndex: number) => void;
}

interface DiffLine {
	prefix: string;
	content: string;
	oldLine: number | null;
	newLine: number | null;
	lineIndex: number;
}

function buildDiffLines(hunk: Hunk): DiffLine[] {
	const lines: DiffLine[] = [];
	let oldLine = hunk.oldStart;
	let newLine = hunk.newStart;

	for (let idx = 0; idx < hunk.lines.length; idx++) {
		const raw = hunk.lines[idx];
		const prefix = raw[0];
		const content = raw.slice(1);

		if (prefix === "\\") continue;

		if (prefix === "-") {
			lines.push({ prefix, content, oldLine, newLine: null, lineIndex: idx });
			oldLine++;
		} else if (prefix === "+") {
			lines.push({
				prefix,
				content,
				oldLine: null,
				newLine,
				lineIndex: idx,
			});
			newLine++;
		} else {
			lines.push({ prefix, content, oldLine, newLine, lineIndex: idx });
			oldLine++;
			newLine++;
		}
	}

	return lines;
}

function lineStyle(prefix: string): string {
	if (prefix === "+") return "bg-diff-added-bg text-diff-added-fg";
	if (prefix === "-") return "bg-diff-deleted-bg text-diff-deleted-fg";
	return "bg-background text-foreground";
}

const LONG_PRESS_MS = 500;

const lineNumberClass =
	"w-12 text-right px-2 text-muted-foreground select-none border-r border-border font-mono text-xs";

function isInRange(lineNum: number, range: LineRange | null): boolean {
	if (!range) return false;
	return lineNum >= range.start && lineNum <= range.end;
}

export function DiffRenderer({
	original,
	modified,
	filePath,
	selectionStart,
	highlightRange,
	onLineTap,
	onLineLongPress,
	changeGroups,
	onStageGroup,
	onUnstageGroup,
}: DiffRendererProps) {
	const hunks = useMemo(
		() => computeHunks(original, modified, filePath),
		[original, modified, filePath],
	);

	const groups = useMemo(() => {
		if (changeGroups) return changeGroups;
		return computeChangeGroups(hunks);
	}, [changeGroups, hunks]);

	const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const longPressedRef = useRef(false);

	const handlePointerDown = useCallback(
		(lineNum: number) => {
			longPressedRef.current = false;
			timerRef.current = setTimeout(() => {
				longPressedRef.current = true;
				onLineLongPress(lineNum);
			}, LONG_PRESS_MS);
		},
		[onLineLongPress],
	);

	const handlePointerUp = useCallback(
		(lineNum: number) => {
			if (timerRef.current) {
				clearTimeout(timerRef.current);
				timerRef.current = null;
			}
			if (!longPressedRef.current) {
				onLineTap(lineNum);
			}
			longPressedRef.current = false;
		},
		[onLineTap],
	);

	const handlePointerCancel = useCallback(() => {
		if (timerRef.current) {
			clearTimeout(timerRef.current);
			timerRef.current = null;
		}
		longPressedRef.current = false;
	}, []);

	if (hunks.length === 0) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground text-sm">
				No changes
			</div>
		);
	}

	return (
		<div className="overflow-x-auto overflow-y-auto h-full">
			<table className="w-full border-collapse text-xs font-mono">
				<tbody>
					{hunks.map((hunk) => {
						const diffLines = buildDiffLines(hunk);
						const hunkGroups = groups.filter((g) => g.hunkIndex === hunk.index);
						return (
							<HunkRows
								key={hunk.index}
								hunk={hunk}
								diffLines={diffLines}
								groups={hunkGroups}
								selectionStart={selectionStart}
								highlightRange={highlightRange}
								onPointerDown={handlePointerDown}
								onPointerUp={handlePointerUp}
								onPointerCancel={handlePointerCancel}
								onStageGroup={onStageGroup}
								onUnstageGroup={onUnstageGroup}
							/>
						);
					})}
				</tbody>
			</table>
		</div>
	);
}

function HunkRows({
	hunk,
	diffLines,
	groups,
	selectionStart,
	highlightRange,
	onPointerDown,
	onPointerUp,
	onPointerCancel,
	onStageGroup,
	onUnstageGroup,
}: {
	hunk: Hunk;
	diffLines: DiffLine[];
	groups: ChangeGroup[];
	selectionStart: number | null;
	highlightRange: LineRange | null;
	onPointerDown: (lineNum: number) => void;
	onPointerUp: (lineNum: number) => void;
	onPointerCancel: () => void;
	onStageGroup?: (groupIndex: number) => void;
	onUnstageGroup?: (groupIndex: number) => void;
}) {
	const groupStartOffsets = useMemo(() => {
		const map = new Map<number, ChangeGroup>();
		for (const g of groups) {
			map.set(g.lineOffsetStart, g);
		}
		return map;
	}, [groups]);

	const hasGroupButtons = onStageGroup != null || onUnstageGroup != null;

	return (
		<>
			<tr className="bg-card/80">
				<td
					colSpan={3}
					className="px-3 py-1 text-muted-foreground text-xs select-none"
				>
					@@ -{hunk.oldStart},{hunk.oldLines} +{hunk.newStart},{hunk.newLines}{" "}
					@@
				</td>
			</tr>
			{diffLines.map((line, i) => {
				const { newLine } = line;
				const tappable = newLine != null;
				const isSelStart = selectionStart != null && newLine === selectionStart;
				const isRangeHighlight = tappable && isInRange(newLine, highlightRange);

				let rowHighlight = "";
				if (isRangeHighlight) {
					rowHighlight = "ring-1 ring-primary bg-primary/10";
				} else if (isSelStart) {
					rowHighlight = "ring-1 ring-warning bg-warning/10";
				}

				const group = hasGroupButtons
					? groupStartOffsets.get(line.lineIndex)
					: undefined;
				const isStaged = group?.isStaged === true;

				return (
					<GroupButtonWrapper
						key={`${hunk.index}-${i}`}
						group={group}
						isStaged={isStaged}
						onStageGroup={onStageGroup}
						onUnstageGroup={onUnstageGroup}
					>
						<tr
							className={`${lineStyle(line.prefix)} select-none ${tappable ? "active:bg-muted/50" : ""} ${rowHighlight} ${isStaged ? "opacity-50" : ""}`}
							onPointerDown={
								tappable ? () => onPointerDown(newLine) : undefined
							}
							onPointerUp={tappable ? () => onPointerUp(newLine) : undefined}
							onPointerLeave={tappable ? onPointerCancel : undefined}
							onPointerCancel={tappable ? onPointerCancel : undefined}
						>
							<td className={lineNumberClass}>{line.oldLine ?? ""}</td>
							<td className={lineNumberClass}>{newLine ?? ""}</td>
							<td className="px-3 py-0 whitespace-pre">
								{line.prefix}
								{line.content}
							</td>
						</tr>
					</GroupButtonWrapper>
				);
			})}
		</>
	);
}

function GroupButtonWrapper({
	group,
	isStaged,
	onStageGroup,
	onUnstageGroup,
	children,
}: {
	group: ChangeGroup | undefined;
	isStaged: boolean;
	onStageGroup?: (groupIndex: number) => void;
	onUnstageGroup?: (groupIndex: number) => void;
	children: React.ReactNode;
}) {
	if (!group) return <>{children}</>;

	return (
		<>
			<tr className="bg-card/60">
				<td colSpan={3} className="px-3 py-0.5 select-none">
					<div className="flex items-center gap-1.5">
						{isStaged && (
							<span className="text-[10px] text-success font-medium">
								Staged
							</span>
						)}
						{onStageGroup && !isStaged && (
							<button
								type="button"
								onClick={() => onStageGroup(group.groupIndex)}
								className="text-[10px] px-1.5 py-0 rounded bg-success/80 hover:bg-success/70 text-success-foreground transition-colors"
							>
								Stage
							</button>
						)}
						{onUnstageGroup && isStaged && (
							<button
								type="button"
								onClick={() => onUnstageGroup(group.groupIndex)}
								className="text-[10px] px-1.5 py-0 rounded bg-warning/80 hover:bg-warning/70 text-warning-foreground transition-colors"
							>
								Unstage
							</button>
						)}
					</div>
				</td>
			</tr>
			{children}
		</>
	);
}
