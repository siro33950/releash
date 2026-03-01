import { Check, Copy, Pencil, Trash2, X } from "lucide-react";
import {
	type KeyboardEvent,
	type MouseEvent,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { MarkdownPreview } from "@/components/panels/MarkdownPreview";
import { formatRelativeTime } from "@/lib/formatRelativeTime";
import type { LineComment } from "@/types/comment";

export interface CommentThreadProps {
	lineNumber: number;
	endLine?: number;
	comments: LineComment[];
	onSubmit: (content: string) => void;
	onCancel: () => void;
	onDeleteComment?: (id: string) => void;
	onUpdateComment?: (id: string, content: string) => void;
	onCopyComment?: (comment: LineComment) => void;
	onResolveComment?: (id: string) => void;
}

function SeverityBadge({ severity }: { severity: string }) {
	return (
		<span className={`comment-thread-severity severity-${severity}`}>
			{severity}
		</span>
	);
}

function CommentItem({
	comment,
	onDelete,
	onUpdate,
	onCopy,
	onResolve,
}: {
	comment: LineComment;
	onDelete?: (id: string) => void;
	onUpdate?: (id: string, content: string) => void;
	onCopy?: (comment: LineComment) => void;
	onResolve?: (id: string) => void;
}) {
	const [editing, setEditing] = useState(false);
	const [editValue, setEditValue] = useState(comment.content);
	const [content, setContent] = useState(comment.content);
	const [removed, setRemoved] = useState(false);
	const editRef = useRef<HTMLTextAreaElement>(null);

	useEffect(() => {
		if (editing && editRef.current) {
			editRef.current.focus();
		}
	}, [editing]);

	if (removed) return null;

	const handleSave = (e: MouseEvent) => {
		e.stopPropagation();
		const trimmed = editValue.trim();
		if (trimmed && trimmed !== comment.content) {
			onUpdate?.(comment.id, trimmed);
			setContent(trimmed);
		}
		setEditing(false);
	};

	const handleCancelEdit = (e: MouseEvent) => {
		e.stopPropagation();
		setEditValue(content);
		setEditing(false);
	};

	const handleEditKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
		e.stopPropagation();
		if (e.key === "Enter" && !e.shiftKey) {
			e.preventDefault();
			const trimmed = editValue.trim();
			if (trimmed && trimmed !== comment.content) {
				onUpdate?.(comment.id, trimmed);
				setContent(trimmed);
			}
			setEditing(false);
		} else if (e.key === "Escape") {
			e.preventDefault();
			setEditValue(content);
			setEditing(false);
		}
	};

	return (
		<div className="comment-thread-item">
			{!editing && (
				<div className="comment-thread-item-meta">
					<div className="comment-thread-item-meta-left">
						{comment.severity && <SeverityBadge severity={comment.severity} />}
						<span className="comment-thread-time">
							{formatRelativeTime(comment.createdAt)}
						</span>
					</div>
					<div className="comment-thread-item-actions">
						{onUpdate && (
							<button
								type="button"
								className="comment-thread-action-btn"
								title="Edit"
								onClick={(e) => {
									e.stopPropagation();
									setEditValue(content);
									setEditing(true);
								}}
							>
								<Pencil className="h-3.5 w-3.5" />
							</button>
						)}
						{onCopy && (
							<button
								type="button"
								className="comment-thread-action-btn"
								title="Copy"
								onClick={(e) => {
									e.stopPropagation();
									onCopy(comment);
								}}
							>
								<Copy className="h-3.5 w-3.5" />
							</button>
						)}
						{onResolve && (
							<button
								type="button"
								className="comment-thread-action-btn comment-thread-action-resolve"
								title="Resolve"
								onClick={(e) => {
									e.stopPropagation();
									onResolve(comment.id);
									setRemoved(true);
								}}
							>
								<Check className="h-3.5 w-3.5" />
							</button>
						)}
						{onDelete && (
							<button
								type="button"
								className="comment-thread-action-btn comment-thread-action-delete"
								title="Delete"
								onClick={(e) => {
									e.stopPropagation();
									onDelete(comment.id);
									setRemoved(true);
								}}
							>
								<Trash2 className="h-3.5 w-3.5" />
							</button>
						)}
					</div>
				</div>
			)}
			{editing ? (
				<>
					<textarea
						ref={editRef}
						className="comment-thread-edit-textarea"
						value={editValue}
						rows={2}
						onChange={(e) => setEditValue(e.target.value)}
						onKeyDown={handleEditKeyDown}
					/>
					<div className="comment-thread-edit-actions">
						<button
							type="button"
							className="comment-thread-action-btn comment-thread-action-save"
							title="Save"
							onClick={handleSave}
						>
							<Check className="h-3.5 w-3.5" />
						</button>
						<button
							type="button"
							className="comment-thread-action-btn"
							title="Cancel"
							onClick={handleCancelEdit}
						>
							<X className="h-3.5 w-3.5" />
						</button>
					</div>
				</>
			) : (
				<div className="comment-thread-item-content">
					<MarkdownPreview
						content={content}
						className="comment-thread-markdown"
					/>
				</div>
			)}
		</div>
	);
}

export function CommentThread({
	lineNumber,
	endLine,
	comments,
	onSubmit,
	onCancel,
	onDeleteComment,
	onUpdateComment,
	onCopyComment,
	onResolveComment,
}: CommentThreadProps) {
	const textareaRef = useRef<HTMLTextAreaElement>(null);

	useEffect(() => {
		textareaRef.current?.focus();
	}, []);

	const handleSubmitClick = useCallback(
		(e: MouseEvent) => {
			e.stopPropagation();
			const content = textareaRef.current?.value.trim();
			if (content) {
				onSubmit(content);
			} else {
				onCancel();
			}
		},
		[onSubmit, onCancel],
	);

	const handleCancelClick = useCallback(
		(e: MouseEvent) => {
			e.stopPropagation();
			onCancel();
		},
		[onCancel],
	);

	const title =
		endLine != null ? `L${lineNumber}-${endLine}` : `L${lineNumber}`;

	return (
		<>
			<div className="comment-thread-header">
				<span className="comment-thread-header-title">{title}</span>
				<button
					type="button"
					className="comment-thread-close-btn"
					onClick={(e) => {
						e.stopPropagation();
						onCancel();
					}}
				>
					&times;
				</button>
			</div>
			{comments.length > 0 && (
				<div className="comment-thread-items">
					{comments.map((comment) => (
						<CommentItem
							key={comment.id}
							comment={comment}
							onDelete={onDeleteComment}
							onUpdate={onUpdateComment}
							onCopy={onCopyComment}
							onResolve={onResolveComment}
						/>
					))}
				</div>
			)}
			<div className="comment-thread-reply">
				<textarea
					ref={textareaRef}
					className="comment-thread-textarea"
					placeholder="返信を入力..."
					rows={3}
				/>
				<div className="comment-thread-actions">
					<button
						type="button"
						className="comment-thread-cancel-btn"
						onClick={handleCancelClick}
					>
						キャンセル
					</button>
					<button
						type="button"
						className="comment-thread-submit-btn"
						onClick={handleSubmitClick}
					>
						追加
					</button>
				</div>
			</div>
		</>
	);
}
