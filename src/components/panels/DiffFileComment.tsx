import { MessageSquare, MessageSquarePlus } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import {
	getThreadFilePath,
	getThreadLineNumber,
	type ReviewDiscussionThread,
} from "@/types/diffComment";
import type { ReviewStanceValue } from "@/types/protocol";
import { DiffInlineComment, DiffInlineCommentInput } from "./DiffInlineComment";

interface FileCommentPopoverTriggerProps {
	comments: ReviewDiscussionThread[];
	filePath?: string;
	title?: string;
	addLabel?: string;
	onAdd: (content: string) => Promise<void>;
	onAppend: (
		threadId: string,
		content: string,
		stance?: ReviewStanceValue | null,
	) => Promise<void>;
	onResolve: (
		threadId: string,
		outcome: string,
		summary: string,
	) => Promise<void>;
	onDelete?: (threadId: string) => Promise<void>;
	/** Controlled mode: 親が open 状態を制御する場合に指定 */
	open?: boolean;
	/** Controlled mode: open 状態が変化したときのコールバック */
	onOpenChange?: (open: boolean) => void;
}

export function FileCommentPopoverTrigger({
	comments,
	filePath,
	title = "File comments",
	addLabel = "Add file comment",
	onAdd,
	onAppend,
	onResolve,
	onDelete,
	open,
	onOpenChange,
}: FileCommentPopoverTriggerProps) {
	const [showInput, setShowInput] = useState(false);

	const fileComments = comments.filter(
		(c) =>
			getThreadLineNumber(c) == null &&
			getThreadFilePath(c) === (filePath ?? ""),
	);
	const hasComments = fileComments.length > 0;

	return (
		<Popover open={open} onOpenChange={onOpenChange}>
			<PopoverTrigger asChild>
				<Button
					variant="ghost"
					size="icon-xs"
					className="h-5 w-5 text-muted-foreground hover:text-foreground relative"
					title={title}
				>
					{hasComments ? (
						<MessageSquare className="h-3.5 w-3.5" />
					) : (
						<MessageSquarePlus className="h-3.5 w-3.5" />
					)}
					{hasComments && (
						<span className="absolute -top-0.5 -right-0.5 size-3 rounded-full bg-blue-600 text-[8px] text-white flex items-center justify-center">
							{fileComments.length}
						</span>
					)}
				</Button>
			</PopoverTrigger>
			<PopoverContent
				align="end"
				side="bottom"
				className="w-80 p-0 max-h-[400px] overflow-auto"
			>
				<div className="flex flex-col">
					{fileComments.map((comment) => (
						<DiffInlineComment
							key={comment.id}
							comment={comment}
							onAppend={onAppend}
							onResolve={onResolve}
							onDelete={onDelete}
						/>
					))}
					{showInput ? (
						<DiffInlineCommentInput
							onSubmit={async (content) => {
								await onAdd(content);
								setShowInput(false);
							}}
							onCancel={() => setShowInput(false)}
						/>
					) : (
						<div className="p-2 border-t border-border">
							<Button
								variant="ghost"
								size="sm"
								onClick={() => setShowInput(true)}
								className="h-7 w-full text-xs text-muted-foreground justify-start"
							>
								<MessageSquarePlus className="size-3 mr-1" />
								{addLabel}
							</Button>
						</div>
					)}
				</div>
			</PopoverContent>
		</Popover>
	);
}
