import { MessageSquare, MessageSquarePlus } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import type { DiffComment } from "@/types/diffComment";
import { DiffInlineComment, DiffInlineCommentInput } from "./DiffInlineComment";

interface FileCommentPopoverTriggerProps {
	comments: DiffComment[];
	filePath: string;
	onAdd: (content: string) => Promise<void>;
	onUpdate: (commentId: string, content: string) => Promise<void>;
	onDelete: (commentId: string) => Promise<void>;
	onSend: (commentIds: string[]) => Promise<void>;
}

export function FileCommentPopoverTrigger({
	comments,
	filePath,
	onAdd,
	onUpdate,
	onDelete,
	onSend,
}: FileCommentPopoverTriggerProps) {
	const [showInput, setShowInput] = useState(false);

	const fileComments = comments.filter(
		(c) => c.lineNumber == null && c.filePath === filePath,
	);
	const hasComments = fileComments.length > 0;

	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button
					variant="ghost"
					size="icon-xs"
					className="h-5 w-5 text-muted-foreground hover:text-foreground relative"
					title="File comments"
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
							onUpdate={onUpdate}
							onDelete={onDelete}
							onSend={onSend}
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
								Add file comment
							</Button>
						</div>
					)}
				</div>
			</PopoverContent>
		</Popover>
	);
}
