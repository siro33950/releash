import { useCallback, useState } from "react";

interface RemoteCommentInputProps {
	lineNumber: number;
	endLine?: number;
	onSave: (content: string) => void;
	onCancel: () => void;
}

export function RemoteCommentInput({
	lineNumber,
	endLine,
	onSave,
	onCancel,
}: RemoteCommentInputProps) {
	const [content, setContent] = useState("");

	const handleSave = useCallback(() => {
		const trimmed = content.trim();
		if (trimmed) {
			onSave(trimmed);
		}
	}, [content, onSave]);

	return (
		<div className="border-t border-neutral-700 bg-neutral-900 p-3 animate-in slide-in-from-bottom-2">
			<div className="flex items-center gap-2 mb-2">
				<span className="text-xs font-mono text-neutral-400 bg-neutral-800 px-1.5 py-0.5 rounded">
					L{lineNumber}{endLine != null ? `-${endLine}` : ""}
				</span>
				<span className="text-xs text-neutral-500">コメントを追加</span>
			</div>
			<textarea
				value={content}
				onChange={(e) => setContent(e.target.value)}
				placeholder="コメントを入力..."
				className="w-full bg-neutral-800 text-neutral-100 text-sm rounded px-3 py-2 resize-none border border-neutral-700 focus:border-blue-500 focus:outline-none"
				rows={3}
			/>
			<div className="flex justify-end gap-2 mt-2">
				<button
					type="button"
					onClick={onCancel}
					className="text-xs px-3 py-1.5 rounded bg-neutral-800 text-neutral-300 hover:bg-neutral-700 transition-colors min-h-[32px]"
				>
					キャンセル
				</button>
				<button
					type="button"
					onClick={handleSave}
					disabled={!content.trim()}
					className="text-xs px-3 py-1.5 rounded bg-blue-600 text-white hover:bg-blue-500 transition-colors disabled:opacity-40 disabled:cursor-not-allowed min-h-[32px]"
				>
					保存
				</button>
			</div>
		</div>
	);
}
