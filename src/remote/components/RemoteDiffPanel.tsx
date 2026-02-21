import { useCallback, useMemo, useState } from "react";
import {
	type ChangeGroup,
	computeChangeGroups,
	computeHunks,
	type Hunk,
	markStagedGroups,
} from "@/lib/computeHunks";
import { generateGroupPatch } from "@/lib/generatePatch";
import type { DiffBase } from "@/remote/hooks/useRemoteFileContent";
import { DiffRenderer } from "./DiffRenderer";
import { RemoteCommentInput } from "./RemoteCommentInput";

interface LineRange {
	start: number;
	end: number;
}

interface RemoteDiffPanelProps {
	path: string | null;
	original: string;
	modified: string;
	loading: boolean;
	diffBase: DiffBase;
	staged: string | null;
	onStageHunk?: (patch: string) => void;
	onAddComment?: (
		filePath: string,
		lineNumber: number,
		content: string,
		endLine?: number,
	) => void;
}

function findMatchingGroup(
	targetLines: string[],
	hunks: Hunk[],
	groups: ChangeGroup[],
	reverse = false,
): { group: ChangeGroup; hunk: Hunk } | null {
	let target: string;
	if (reverse) {
		const newMinus = targetLines
			.filter((l) => l.startsWith("+"))
			.map((l) => `-${l.slice(1)}`);
		const newPlus = targetLines
			.filter((l) => l.startsWith("-"))
			.map((l) => `+${l.slice(1)}`);
		target = [...newMinus, ...newPlus].join("\n");
	} else {
		target = targetLines.join("\n");
	}
	for (const g of groups) {
		const h = hunks.find((h) => h.index === g.hunkIndex);
		if (!h) continue;
		const lines = h.lines
			.slice(g.lineOffsetStart, g.lineOffsetEnd + 1)
			.join("\n");
		if (lines === target) return { group: g, hunk: h };
	}
	return null;
}

export function RemoteDiffPanel({
	path,
	original,
	modified,
	loading,
	diffBase,
	staged,
	onStageHunk,
	onAddComment,
}: RemoteDiffPanelProps) {
	const [selectionStart, setSelectionStart] = useState<number | null>(null);
	const [commentRange, setCommentRange] = useState<LineRange | null>(null);

	const hunks = useMemo(
		() => (path ? computeHunks(original, modified, path) : []),
		[original, modified, path],
	);

	const stagedHunks = useMemo(() => {
		if (!path || staged == null || diffBase !== "HEAD") return [];
		return computeHunks(original, staged, path);
	}, [original, staged, diffBase, path]);

	const changeGroups = useMemo(() => {
		const groups = computeChangeGroups(hunks);
		if (diffBase !== "HEAD" || stagedHunks.length === 0) return groups;
		const sGroups = computeChangeGroups(stagedHunks);
		return markStagedGroups(groups, sGroups, hunks, stagedHunks);
	}, [hunks, stagedHunks, diffBase]);

	const handleStageGroup = useCallback(
		(groupIndex: number) => {
			if (!path || !onStageHunk) return;
			const group = changeGroups.find((g) => g.groupIndex === groupIndex);
			if (!group) return;
			const hunk = hunks.find((h) => h.index === group.hunkIndex);
			if (!hunk) return;

			let patchHunk = hunk;
			let patchGroup = group;

			if (diffBase === "HEAD" && staged != null) {
				const targetLines = hunk.lines.slice(
					group.lineOffsetStart,
					group.lineOffsetEnd + 1,
				);
				const s2wHunks = computeHunks(staged, modified, path);
				const s2wGroups = computeChangeGroups(s2wHunks);
				const match = findMatchingGroup(targetLines, s2wHunks, s2wGroups);
				if (!match) return;
				patchHunk = match.hunk;
				patchGroup = match.group;
			}

			const patch = generateGroupPatch(path, patchHunk, patchGroup);
			if (patch) onStageHunk(patch);
		},
		[path, onStageHunk, changeGroups, hunks, diffBase, staged, modified],
	);

	const handleUnstageGroup = useCallback(
		(groupIndex: number) => {
			if (!path || !onStageHunk || staged == null) return;
			const group = changeGroups.find((g) => g.groupIndex === groupIndex);
			if (!group) return;
			const hunk = hunks.find((h) => h.index === group.hunkIndex);
			if (!hunk) return;

			const targetLines = hunk.lines.slice(
				group.lineOffsetStart,
				group.lineOffsetEnd + 1,
			);
			const s2hHunks = computeHunks(staged, original, path);
			const s2hGroups = computeChangeGroups(s2hHunks);
			const match = findMatchingGroup(targetLines, s2hHunks, s2hGroups, true);
			if (!match) return;

			const patch = generateGroupPatch(path, match.hunk, match.group);
			if (patch) onStageHunk(patch);
		},
		[path, onStageHunk, changeGroups, hunks, staged, original],
	);

	const handleLineTap = useCallback(
		(lineNumber: number) => {
			if (!onAddComment) return;

			if (selectionStart != null) {
				const start = Math.min(selectionStart, lineNumber);
				const end = Math.max(selectionStart, lineNumber);
				setCommentRange({ start, end });
				setSelectionStart(null);
			} else {
				setCommentRange({ start: lineNumber, end: lineNumber });
			}
		},
		[onAddComment, selectionStart],
	);

	const handleLineLongPress = useCallback(
		(lineNumber: number) => {
			if (!onAddComment) return;
			setSelectionStart(lineNumber);
			setCommentRange(null);
		},
		[onAddComment],
	);

	const handleSaveComment = useCallback(
		(content: string) => {
			if (path && commentRange) {
				const endLine =
					commentRange.start !== commentRange.end
						? commentRange.end
						: undefined;
				onAddComment?.(path, commentRange.start, content, endLine);
			}
			setCommentRange(null);
		},
		[path, commentRange, onAddComment],
	);

	const handleCancelComment = useCallback(() => {
		setCommentRange(null);
		setSelectionStart(null);
	}, []);

	if (!path) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground text-sm">
				Select a file to view diff
			</div>
		);
	}

	if (loading) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground text-sm">
				Loading...
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full">
			{selectionStart != null && (
				<div className="flex items-center px-3 py-1 border-b border-warning/50 bg-warning/10 shrink-0">
					<span className="text-xs text-warning">
						L{selectionStart} から範囲選択中 — 終了行をタップ
					</span>
					<button
						type="button"
						onClick={() => setSelectionStart(null)}
						className="ml-auto text-xs px-2 py-0.5 rounded bg-secondary text-secondary-foreground hover:bg-secondary/80 transition-colors"
					>
						キャンセル
					</button>
				</div>
			)}
			<div className="flex-1" style={{ minHeight: 0 }}>
				<DiffRenderer
					key={path}
					original={original}
					modified={modified}
					filePath={path}
					selectionStart={selectionStart}
					highlightRange={commentRange}
					onLineTap={handleLineTap}
					onLineLongPress={handleLineLongPress}
					changeGroups={changeGroups}
					onStageGroup={onStageHunk ? handleStageGroup : undefined}
					onUnstageGroup={
						onStageHunk && diffBase === "HEAD" ? handleUnstageGroup : undefined
					}
				/>
			</div>
			{commentRange != null && (
				<RemoteCommentInput
					lineNumber={commentRange.start}
					endLine={
						commentRange.start !== commentRange.end
							? commentRange.end
							: undefined
					}
					onSave={handleSaveComment}
					onCancel={handleCancelComment}
				/>
			)}
		</div>
	);
}
