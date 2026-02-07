import { useCallback, useMemo, useRef } from "react";
import { computeHunks, type Hunk } from "@/lib/computeHunks";

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
}

interface DiffLine {
	prefix: string;
	content: string;
	oldLine: number | null;
	newLine: number | null;
}

function buildDiffLines(hunk: Hunk): DiffLine[] {
	const lines: DiffLine[] = [];
	let oldLine = hunk.oldStart;
	let newLine = hunk.newStart;

	for (const raw of hunk.lines) {
		const prefix = raw[0];
		const content = raw.slice(1);

		if (prefix === "\\") continue;

		if (prefix === "-") {
			lines.push({ prefix, content, oldLine, newLine: null });
			oldLine++;
		} else if (prefix === "+") {
			lines.push({ prefix, content, oldLine: null, newLine });
			newLine++;
		} else {
			lines.push({ prefix, content, oldLine, newLine });
			oldLine++;
			newLine++;
		}
	}

	return lines;
}

function lineStyle(prefix: string): string {
	if (prefix === "+") return "bg-green-950/40 text-green-300";
	if (prefix === "-") return "bg-red-950/40 text-red-300";
	return "bg-neutral-950 text-neutral-300";
}

const LONG_PRESS_MS = 500;

const lineNumberClass =
	"w-12 text-right px-2 text-neutral-600 select-none border-r border-neutral-800 font-mono text-xs";

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
}: DiffRendererProps) {
	const hunks = useMemo(
		() => computeHunks(original, modified, filePath),
		[original, modified, filePath],
	);

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
			<div className="flex items-center justify-center h-full text-neutral-500 text-sm">
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
						return (
							<HunkRows
								key={hunk.index}
								hunk={hunk}
								diffLines={diffLines}
								selectionStart={selectionStart}
								highlightRange={highlightRange}
								onPointerDown={handlePointerDown}
								onPointerUp={handlePointerUp}
								onPointerCancel={handlePointerCancel}
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
	selectionStart,
	highlightRange,
	onPointerDown,
	onPointerUp,
	onPointerCancel,
}: {
	hunk: Hunk;
	diffLines: DiffLine[];
	selectionStart: number | null;
	highlightRange: LineRange | null;
	onPointerDown: (lineNum: number) => void;
	onPointerUp: (lineNum: number) => void;
	onPointerCancel: () => void;
}) {
	return (
		<>
			<tr className="bg-neutral-900/80">
				<td
					colSpan={3}
					className="px-3 py-1 text-neutral-500 text-xs select-none"
				>
					@@ -{hunk.oldStart},{hunk.oldLines} +{hunk.newStart},{hunk.newLines}{" "}
					@@
				</td>
			</tr>
			{diffLines.map((line, i) => {
				const tappable = line.newLine != null;
				const isSelStart =
					selectionStart != null && line.newLine === selectionStart;
				const isRangeHighlight =
					tappable && isInRange(line.newLine!, highlightRange);

				let rowHighlight = "";
				if (isRangeHighlight) {
					rowHighlight = "ring-1 ring-blue-500 bg-blue-950/30";
				} else if (isSelStart) {
					rowHighlight = "ring-1 ring-amber-500 bg-amber-950/30";
				}

				return (
					<tr
						key={`${hunk.index}-${i}`}
						className={`${lineStyle(line.prefix)} select-none ${tappable ? "active:bg-neutral-700/50" : ""} ${rowHighlight}`}
						onPointerDown={tappable ? () => onPointerDown(line.newLine!) : undefined}
						onPointerUp={tappable ? () => onPointerUp(line.newLine!) : undefined}
						onPointerLeave={tappable ? onPointerCancel : undefined}
						onPointerCancel={tappable ? onPointerCancel : undefined}
					>
						<td className={lineNumberClass}>
							{line.oldLine ?? ""}
						</td>
						<td className={lineNumberClass}>
							{line.newLine ?? ""}
						</td>
						<td className="px-3 py-0 whitespace-pre">
							{line.prefix}
							{line.content}
						</td>
					</tr>
				);
			})}
		</>
	);
}
