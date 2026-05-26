import { CheckCircle2, MessageSquare } from "lucide-react";
import { useMemo } from "react";
import { EmptyState } from "@/components/ui/empty-state";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import {
	getThreadEndLine,
	getThreadFilePath,
	getThreadLineNumber,
	getThreadPreviewContent,
	type ReviewDiscussionThread,
} from "@/types/diffComment";

export interface DiffCommentListProps {
	comments: ReviewDiscussionThread[];
	onCommentClick: (filePath: string, lineNumber?: number) => void;
}

function formatLineLabel(comment: ReviewDiscussionThread): string {
	const lineNumber = getThreadLineNumber(comment);
	const endLine = getThreadEndLine(comment);
	if (lineNumber == null) return "file";
	if (endLine != null && endLine !== lineNumber) {
		return `L${lineNumber}-${endLine}`;
	}
	return `L${lineNumber}`;
}

function getFileName(filePath: string): string {
	const parts = filePath.split("/");
	return parts[parts.length - 1] ?? filePath;
}

export function DiffCommentList({
	comments,
	onCommentClick,
}: DiffCommentListProps) {
	const groupedByFile = useMemo(() => {
		const map = new Map<string, ReviewDiscussionThread[]>();
		for (const comment of comments) {
			const filePath = getThreadFilePath(comment);
			const existing = map.get(filePath);
			if (existing) {
				existing.push(comment);
			} else {
				map.set(filePath, [comment]);
			}
		}
		return map;
	}, [comments]);

	if (comments.length === 0) {
		return (
			<div className="h-full flex flex-col">
				<div className="flex items-center justify-between px-3 py-1.5 border-b border-border shrink-0">
					<span className="text-xs font-medium text-foreground">Threads</span>
				</div>
				<div className="flex-1 flex items-center justify-center">
					<EmptyState
						icon={MessageSquare}
						title="No threads yet"
						description="Add comments on diff lines"
					/>
				</div>
			</div>
		);
	}

	return (
		<div className="h-full flex flex-col">
			<div className="flex items-center justify-between px-3 py-1.5 border-b border-border shrink-0">
				<span className="text-xs font-medium text-foreground">Threads</span>
			</div>
			<div className="flex-1 min-h-0 overflow-auto">
				<div className="px-2 py-1">
					{[...groupedByFile.entries()].map(([filePath, fileComments]) => (
						<div key={filePath || "general"} className="mb-2">
							<Tooltip>
								<TooltipTrigger asChild>
									<button
										type="button"
										onClick={() => onCommentClick(filePath)}
										className="w-full text-left px-1 py-0.5 text-[11px] font-medium text-muted-foreground hover:text-foreground truncate"
									>
										{filePath ? getFileName(filePath) : "General"}
									</button>
								</TooltipTrigger>
								<TooltipContent side="top" className="text-xs">
									{filePath || "General"}
								</TooltipContent>
							</Tooltip>
							<div className="space-y-0.5">
								{fileComments.map((comment) => (
									<button
										type="button"
										key={comment.id}
										onClick={() =>
											onCommentClick(
												getThreadFilePath(comment),
												getThreadLineNumber(comment),
											)
										}
										className="group/item w-full text-left flex items-start gap-1.5 px-1.5 py-1 rounded hover:bg-muted/50 transition-colors"
									>
										<span className="shrink-0 text-[10px] font-mono text-muted-foreground mt-0.5 min-w-[36px]">
											{formatLineLabel(comment)}
										</span>
										<span className="flex-1 text-xs text-foreground truncate">
											{getThreadPreviewContent(comment)}
										</span>
										<span className="shrink-0 text-[10px] text-muted-foreground">
											{comment.comments.length}
										</span>
										{comment.state === "resolved" && (
											<span className="shrink-0 inline-flex items-center gap-0.5 text-[9px] bg-green-600/15 text-green-600 px-1 py-0.5 rounded-full">
												<CheckCircle2 className="h-2.5 w-2.5" />
												resolved
											</span>
										)}
									</button>
								))}
							</div>
						</div>
					))}
				</div>
			</div>
		</div>
	);
}
