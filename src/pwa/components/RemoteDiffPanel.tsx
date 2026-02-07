import { useCallback, useState } from "react";
import { RemoteCommentInput } from "./RemoteCommentInput";
import { DiffRenderer } from "./DiffRenderer";

interface LineRange {
	start: number;
	end: number;
}

interface RemoteDiffPanelProps {
	path: string | null;
	original: string;
	modified: string;
	loading: boolean;
	onAddComment?: (filePath: string, lineNumber: number, content: string, endLine?: number) => void;
}

export function RemoteDiffPanel({
	path,
	original,
	modified,
	loading,
	onAddComment,
}: RemoteDiffPanelProps) {
	const [selectionStart, setSelectionStart] = useState<number | null>(null);
	const [commentRange, setCommentRange] = useState<LineRange | null>(null);

	const handleLineTap = useCallback((lineNumber: number) => {
		if (!onAddComment) return;

		if (selectionStart != null) {
			const start = Math.min(selectionStart, lineNumber);
			const end = Math.max(selectionStart, lineNumber);
			setCommentRange({ start, end });
			setSelectionStart(null);
		} else {
			setCommentRange({ start: lineNumber, end: lineNumber });
		}
	}, [onAddComment, selectionStart]);

	const handleLineLongPress = useCallback((lineNumber: number) => {
		if (!onAddComment) return;
		setSelectionStart(lineNumber);
		setCommentRange(null);
	}, [onAddComment]);

	const handleSaveComment = useCallback(
		(content: string) => {
			if (path && commentRange) {
				const endLine = commentRange.start !== commentRange.end
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
			<div className="flex items-center justify-center h-full text-neutral-500 text-sm">
				Select a file to view diff
			</div>
		);
	}

	if (loading) {
		return (
			<div className="flex items-center justify-center h-full text-neutral-500 text-sm">
				Loading...
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full">
			{selectionStart != null && (
				<div className="flex items-center px-3 py-1 border-b border-amber-800/50 bg-amber-950/30 shrink-0">
					<span className="text-xs text-amber-400">
						L{selectionStart} から範囲選択中 — 終了行をタップ
					</span>
					<button
						type="button"
						onClick={() => setSelectionStart(null)}
						className="ml-auto text-xs px-2 py-0.5 rounded bg-neutral-800 text-neutral-300 hover:bg-neutral-700 transition-colors"
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
				/>
			</div>
			{commentRange != null && (
				<RemoteCommentInput
					lineNumber={commentRange.start}
					endLine={commentRange.start !== commentRange.end ? commentRange.end : undefined}
					onSave={handleSaveComment}
					onCancel={handleCancelComment}
				/>
			)}
		</div>
	);
}
