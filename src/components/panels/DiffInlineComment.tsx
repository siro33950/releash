import { Check, Pencil, Send, Trash2, X } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import type { DiffComment } from "@/types/diffComment";

interface DiffInlineCommentProps {
	comment: DiffComment;
	onUpdate: (commentId: string, content: string) => Promise<void>;
	onDelete: (commentId: string) => Promise<void>;
	onSend: (commentIds: string[]) => Promise<void>;
}

export function DiffInlineComment({
	comment,
	onUpdate,
	onDelete,
	onSend,
}: DiffInlineCommentProps) {
	const [editing, setEditing] = useState(false);
	const [editContent, setEditContent] = useState(comment.content);

	const handleSave = async () => {
		if (editContent.trim() === "") return;
		await onUpdate(comment.id, editContent.trim());
		setEditing(false);
	};

	const handleCancel = () => {
		setEditContent(comment.content);
		setEditing(false);
	};

	const rangeLabel =
		comment.lineNumber && comment.endLine
			? `L${comment.lineNumber}-${comment.endLine}`
			: comment.lineNumber
				? `L${comment.lineNumber}`
				: null;

	return (
		<div className="flex flex-col gap-1.5 mx-2 my-1 p-3 rounded-md border border-border bg-card shadow-sm">
			{rangeLabel && (
				<span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground font-mono bg-muted px-1.5 py-0.5 rounded w-fit">
					{rangeLabel}
				</span>
			)}

			{editing ? (
				<div className="flex flex-col gap-1">
					<Textarea
						value={editContent}
						onChange={(e) => setEditContent(e.target.value)}
						className="min-h-[80px] text-sm resize-none bg-background"
						autoFocus
						onKeyDown={(e) => {
							if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
								handleSave();
							}
							if (e.key === "Escape") {
								handleCancel();
							}
						}}
					/>
					<div className="flex gap-1 justify-end">
						<Button
							variant="ghost"
							size="sm"
							onClick={handleCancel}
							className="h-7 px-3 text-xs"
						>
							<X className="size-3 mr-1" />
							Cancel
						</Button>
						<Button
							variant="default"
							size="sm"
							onClick={handleSave}
							className="h-7 px-3 text-xs"
							disabled={editContent.trim() === ""}
						>
							<Check className="size-3 mr-1" />
							Save
						</Button>
					</div>
				</div>
			) : (
				<div className="flex items-start gap-2">
					<p className="flex-1 text-sm whitespace-pre-wrap break-words leading-relaxed">
						{comment.content}
					</p>
					<div className="flex items-center gap-0.5 shrink-0">
						{comment.status === "sent" && (
							<span className="text-[10px] bg-green-600/15 text-green-600 px-1.5 py-0.5 rounded-full mr-1">
								sent
							</span>
						)}
						<Button
							variant="ghost"
							size="sm"
							onClick={() => setEditing(true)}
							className="size-6 p-0"
							title="Edit"
						>
							<Pencil className="size-3" />
						</Button>
						<Button
							variant="ghost"
							size="sm"
							onClick={() => onDelete(comment.id)}
							className="size-6 p-0 text-destructive"
							title="Delete"
						>
							<Trash2 className="size-3" />
						</Button>
						{comment.status === "unsent" && (
							<Button
								variant="ghost"
								size="sm"
								onClick={() => onSend([comment.id])}
								className="size-6 p-0"
								title="Send to Agent"
							>
								<Send className="size-3" />
							</Button>
						)}
					</div>
				</div>
			)}
		</div>
	);
}

interface DiffInlineCommentInputProps {
	onSubmit: (content: string) => Promise<void>;
	onCancel: () => void;
	rangeLabel?: string;
}

export function DiffInlineCommentInput({
	onSubmit,
	onCancel,
	rangeLabel,
}: DiffInlineCommentInputProps) {
	const [content, setContent] = useState("");

	const handleSubmit = async () => {
		if (content.trim() === "") return;
		await onSubmit(content.trim());
		setContent("");
	};

	return (
		<div className="flex flex-col gap-2 mx-2 my-1 p-3 rounded-md border border-border bg-card shadow-sm">
			{rangeLabel && (
				<span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground font-mono bg-muted px-1.5 py-0.5 rounded w-fit">
					{rangeLabel}
				</span>
			)}
			<Textarea
				value={content}
				onChange={(e) => setContent(e.target.value)}
				placeholder="Leave a comment..."
				className="min-h-[80px] text-sm resize-none bg-background"
				autoFocus
				onKeyDown={(e) => {
					if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
						handleSubmit();
					}
					if (e.key === "Escape") {
						onCancel();
					}
				}}
			/>
			<div className="flex gap-1 justify-end">
				<Button
					variant="ghost"
					size="sm"
					onClick={onCancel}
					className="h-7 px-3 text-xs"
				>
					Cancel
				</Button>
				<Button
					variant="default"
					size="sm"
					onClick={handleSubmit}
					className="h-7 px-3 text-xs"
					disabled={content.trim() === ""}
				>
					Comment
				</Button>
			</div>
		</div>
	);
}
