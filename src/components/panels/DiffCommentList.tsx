import { MessageSquare, Send, Trash2 } from "lucide-react";
import { useMemo } from "react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { DiffComment } from "@/types/diffComment";

export interface DiffCommentListProps {
	comments: DiffComment[];
	unsentCount: number;
	onCommentClick: (filePath: string, lineNumber?: number) => void;
	onDelete: (commentId: string) => Promise<void>;
	onSend: (commentIds: string[]) => Promise<void>;
	onSendAll: () => Promise<void>;
}

function formatLineLabel(comment: DiffComment): string {
	if (comment.lineNumber == null) return "file";
	if (comment.lineNumber == null) return "";
	if (comment.endLine != null && comment.endLine !== comment.lineNumber) {
		return `L${comment.lineNumber}-${comment.endLine}`;
	}
	return `L${comment.lineNumber}`;
}

function getFileName(filePath: string): string {
	const parts = filePath.split("/");
	return parts[parts.length - 1] ?? filePath;
}

export function DiffCommentList({
	comments,
	unsentCount,
	onCommentClick,
	onDelete,
	onSend,
	onSendAll,
}: DiffCommentListProps) {
	const groupedByFile = useMemo(() => {
		const map = new Map<string, DiffComment[]>();
		for (const comment of comments) {
			const existing = map.get(comment.filePath);
			if (existing) {
				existing.push(comment);
			} else {
				map.set(comment.filePath, [comment]);
			}
		}
		return map;
	}, [comments]);

	if (comments.length === 0) {
		return (
			<div className="h-full flex flex-col">
				<div className="flex items-center justify-between px-3 py-1.5 border-b border-border shrink-0">
					<span className="text-xs font-medium text-foreground">Comments</span>
				</div>
				<div className="flex-1 flex items-center justify-center">
					<EmptyState
						icon={MessageSquare}
						title="No comments yet"
						description="Add comments on diff lines"
					/>
				</div>
			</div>
		);
	}

	return (
		<div className="h-full flex flex-col">
			<div className="flex items-center justify-between px-3 py-1.5 border-b border-border shrink-0">
				<div className="flex items-center gap-1.5">
					<span className="text-xs font-medium text-foreground">Comments</span>
					{unsentCount > 0 && (
						<span className="min-w-[16px] h-[16px] rounded-full bg-blue-600 text-[10px] text-white flex items-center justify-center px-1">
							{unsentCount}
						</span>
					)}
				</div>
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon-xs"
							onClick={() => onSendAll()}
							disabled={unsentCount === 0}
							className="h-5 w-5 text-muted-foreground hover:text-foreground disabled:opacity-30"
							aria-label="Send all unsent comments"
						>
							<Send className="h-3 w-3" />
						</Button>
					</TooltipTrigger>
					<TooltipContent side="bottom" className="text-xs">
						{unsentCount > 0
							? `Send ${unsentCount} comments to Agent`
							: "No unsent comments"}
					</TooltipContent>
				</Tooltip>
			</div>
			<ScrollArea className="flex-1 min-h-0">
				<div className="px-2 py-1">
					{[...groupedByFile.entries()].map(([filePath, fileComments]) => (
						<div key={filePath} className="mb-2">
							<Tooltip>
								<TooltipTrigger asChild>
									<button
										type="button"
										onClick={() => onCommentClick(filePath)}
										className="w-full text-left px-1 py-0.5 text-[11px] font-medium text-muted-foreground hover:text-foreground truncate"
									>
										{getFileName(filePath)}
									</button>
								</TooltipTrigger>
								<TooltipContent side="top" className="text-xs">
									{filePath}
								</TooltipContent>
							</Tooltip>
							<div className="space-y-0.5">
								{fileComments.map((comment) => (
									<button
										type="button"
										key={comment.id}
										onClick={() =>
											onCommentClick(
												comment.filePath,
												comment.lineNumber ?? undefined,
											)
										}
										className="group/item w-full text-left flex items-start gap-1.5 px-1.5 py-1 rounded hover:bg-muted/50 transition-colors"
									>
										<span className="shrink-0 text-[10px] font-mono text-muted-foreground mt-0.5 min-w-[36px]">
											{formatLineLabel(comment)}
										</span>
										<span className="flex-1 text-xs text-foreground truncate">
											{comment.content}
										</span>
										<span className="shrink-0 flex items-center gap-0.5">
											{comment.status === "sent" ? (
												<span className="text-[9px] bg-green-600/15 text-green-600 px-1 py-0.5 rounded-full">
													sent
												</span>
											) : (
												<Button
													variant="ghost"
													size="icon-xs"
													onClick={(e) => {
														e.stopPropagation();
														onSend([comment.id]);
													}}
													className={cn(
														"h-4 w-4 text-muted-foreground hover:text-foreground",
														"opacity-0 group-hover/item:opacity-100",
													)}
													aria-label="Send comment"
												>
													<Send className="h-2.5 w-2.5" />
												</Button>
											)}
											<Button
												variant="ghost"
												size="icon-xs"
												onClick={(e) => {
													e.stopPropagation();
													onDelete(comment.id);
												}}
												className={cn(
													"h-4 w-4 text-muted-foreground hover:text-destructive",
													"opacity-0 group-hover/item:opacity-100",
												)}
												aria-label="Delete comment"
											>
												<Trash2 className="h-2.5 w-2.5" />
											</Button>
										</span>
									</button>
								))}
							</div>
						</div>
					))}
				</div>
			</ScrollArea>
		</div>
	);
}
