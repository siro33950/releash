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
		<div className="border-t border-border bg-card p-3 animate-in slide-in-from-bottom-2">
			<div className="flex items-center gap-2 mb-2">
				<span className="text-xs font-mono text-muted-foreground bg-muted px-1.5 py-0.5 rounded">
					L{lineNumber}
					{endLine != null ? `-${endLine}` : ""}
				</span>
				<span className="text-xs text-muted-foreground">コメントを追加</span>
			</div>
			<textarea
				value={content}
				onChange={(e) => setContent(e.target.value)}
				placeholder="コメントを入力..."
				className="w-full bg-input text-foreground text-sm rounded px-3 py-2 resize-none border border-border focus:border-primary focus:outline-none"
				rows={3}
			/>
			<div className="flex justify-end gap-2 mt-2">
				<button
					type="button"
					onClick={onCancel}
					className="text-xs px-3 py-1.5 rounded bg-secondary text-secondary-foreground hover:bg-secondary/80 transition-colors min-h-[32px]"
				>
					キャンセル
				</button>
				<button
					type="button"
					onClick={handleSave}
					disabled={!content.trim()}
					className="text-xs px-3 py-1.5 rounded bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-40 disabled:cursor-not-allowed min-h-[32px]"
				>
					保存
				</button>
			</div>
		</div>
	);
}
