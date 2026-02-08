import { MessageSquare, Send } from "lucide-react";
import type { LineComment } from "@/types/comment";

interface RemoteCommentListProps {
	comments: LineComment[];
	onSendToTerminal?: (comments: LineComment[]) => void;
}

export function RemoteCommentList({
	comments,
	onSendToTerminal,
}: RemoteCommentListProps) {
	const unsentComments = comments.filter((c) => c.status === "unsent");

	if (comments.length === 0) {
		return (
			<div className="flex flex-col items-center justify-center h-full gap-3 text-neutral-500 px-6">
				<MessageSquare className="h-8 w-8" />
				<span className="text-sm font-medium">コメントなし</span>
				<p className="text-xs text-center leading-relaxed">
					Diff画面のコメントボタンからコメントを追加できます
				</p>
			</div>
		);
	}

	const grouped = new Map<string, LineComment[]>();
	for (const comment of comments) {
		const existing = grouped.get(comment.filePath);
		if (existing) {
			existing.push(comment);
		} else {
			grouped.set(comment.filePath, [comment]);
		}
	}

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-between px-3 py-2 border-b border-neutral-800 bg-neutral-900 shrink-0">
				<span className="text-xs text-neutral-400">
					Comments
					{unsentComments.length > 0 && (
						<span className="ml-1.5 px-1.5 py-0.5 text-[10px] bg-blue-500/20 text-blue-400 rounded">
							{unsentComments.length}
						</span>
					)}
				</span>
				{unsentComments.length > 0 && onSendToTerminal && (
					<button
						type="button"
						onClick={() => onSendToTerminal(unsentComments)}
						className="flex items-center gap-1.5 px-3 py-1.5 text-xs bg-blue-500/20 text-blue-400 rounded hover:bg-blue-500/30 transition-colors min-h-[32px]"
					>
						<Send className="h-3.5 w-3.5" />
						送信
					</button>
				)}
			</div>
			<div className="flex-1 overflow-y-auto p-2">
				{[...grouped.entries()].map(([filePath, fileComments]) => {
					const fileName = filePath.split("/").pop() ?? filePath;
					return (
						<div key={filePath} className="mb-3">
							<div className="text-xs font-medium px-2 py-1 text-neutral-300 truncate">
								{fileName}
							</div>
							{fileComments
								.sort((a, b) => a.lineNumber - b.lineNumber)
								.map((comment) => (
									<div
										key={comment.id}
										className="flex items-start gap-2 px-2 py-2 text-sm rounded hover:bg-neutral-800/50 transition-colors"
									>
										<MessageSquare className="h-4 w-4 shrink-0 mt-0.5 text-neutral-500" />
										<div className="min-w-0 flex-1">
											<div className="flex items-center gap-1.5">
												<span className="text-neutral-500 font-mono text-xs">
													L{comment.lineNumber}
													{comment.endLine != null ? `-${comment.endLine}` : ""}
												</span>
												<span
													className={`text-[10px] px-1 rounded ${
														comment.status === "sent"
															? "bg-green-500/20 text-green-400"
															: "bg-neutral-700 text-neutral-400"
													}`}
												>
													{comment.status === "sent" ? "sent" : "unsent"}
												</span>
											</div>
											<div className="text-neutral-200 mt-0.5 break-words">
												{comment.content}
											</div>
										</div>
									</div>
								))}
						</div>
					);
				})}
			</div>
		</div>
	);
}
