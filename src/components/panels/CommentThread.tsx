import {
	Check,
	Copy,
	Loader2,
	MessageSquareShare,
	Pencil,
	Play,
	ScrollText,
	Trash2,
	X,
} from "lucide-react";
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
import type { Thread, ThreadEntry } from "@/types/thread";

export interface CommentThreadProps {
	thread: Thread;
	onSubmit: (content: string) => void;
	onCancel: () => void;
	onDeleteThread?: (threadId: string) => void;
	onUpdateEntry?: (threadId: string, entryId: string, content: string) => void;
	onCopyThread?: (thread: Thread) => void;
	onResolveThread?: (threadId: string) => void;
	onImplementThread?: (threadId: string) => void;
	onPostToPr?: (threadId: string) => void;
	aiRunningThreadIds?: Set<string>;
	aiTaskThreadIds?: Set<string>;
	onOpenThreadAIModal?: (threadId?: string) => void;
}

function SeverityBadge({ severity }: { severity: string }) {
	return (
		<span className={`comment-thread-severity severity-${severity}`}>
			{severity}
		</span>
	);
}

function EntryItem({
	entry,
	threadId,
	isFirstEntry,
	severity,
	thread,
	onDelete,
	onUpdate,
	onCopy,
	onResolve,
}: {
	entry: ThreadEntry;
	threadId: string;
	isFirstEntry: boolean;
	severity?: string;
	thread: Thread;
	onDelete?: (threadId: string) => void;
	onUpdate?: (threadId: string, entryId: string, content: string) => void;
	onCopy?: (thread: Thread) => void;
	onResolve?: (threadId: string) => void;
}) {
	const [editing, setEditing] = useState(false);
	const [editValue, setEditValue] = useState(entry.content);
	const [content, setContent] = useState(entry.content);
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
		if (trimmed && trimmed !== content) {
			onUpdate?.(threadId, entry.id, trimmed);
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
			if (trimmed && trimmed !== content) {
				onUpdate?.(threadId, entry.id, trimmed);
				setContent(trimmed);
			}
			setEditing(false);
		} else if (e.key === "Escape") {
			e.preventDefault();
			setEditValue(content);
			setEditing(false);
		}
	};

	const isAction = entry.action != null;
	const label = entry.isAi
		? (entry.authorName ?? "AI")
		: (entry.authorName ?? undefined);

	return (
		<div
			className={`comment-thread-item${isAction ? " comment-thread-action-entry" : ""}`}
		>
			{!editing && (
				<div className="comment-thread-item-meta">
					<div className="comment-thread-item-meta-left">
						{isFirstEntry && severity && <SeverityBadge severity={severity} />}
						{entry.isAi && <span className="comment-thread-ai-badge">AI</span>}
						{label && <span className="comment-thread-author">{label}</span>}
						<span className="comment-thread-time">
							{formatRelativeTime(entry.createdAt)}
						</span>
					</div>
					<div className="comment-thread-item-actions">
						{onUpdate && !isAction && (
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
						{isFirstEntry && onCopy && (
							<button
								type="button"
								className="comment-thread-action-btn"
								title="Copy"
								onClick={(e) => {
									e.stopPropagation();
									onCopy(thread);
								}}
							>
								<Copy className="h-3.5 w-3.5" />
							</button>
						)}
						{isFirstEntry && onResolve && (
							<button
								type="button"
								className="comment-thread-action-btn comment-thread-action-resolve"
								title="Resolve"
								onClick={(e) => {
									e.stopPropagation();
									onResolve(threadId);
									setRemoved(true);
								}}
							>
								<Check className="h-3.5 w-3.5" />
							</button>
						)}
						{isFirstEntry && onDelete && (
							<button
								type="button"
								className="comment-thread-action-btn comment-thread-action-delete"
								title="Delete"
								onClick={(e) => {
									e.stopPropagation();
									onDelete(threadId);
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
	thread,
	onSubmit,
	onCancel,
	onDeleteThread,
	onUpdateEntry,
	onCopyThread,
	onResolveThread,
	onImplementThread,
	onPostToPr,
	aiRunningThreadIds,
	aiTaskThreadIds,
	onOpenThreadAIModal,
}: CommentThreadProps) {
	const textareaRef = useRef<HTMLTextAreaElement>(null);

	useEffect(() => {
		textareaRef.current?.focus();
	}, []);

	const handleSend = useCallback(
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
		thread.endLine != null
			? `L${thread.lineNumber}-${thread.endLine}`
			: `L${thread.lineNumber}`;

	const sortedEntries = [...thread.entries].sort(
		(a, b) => a.createdAt - b.createdAt,
	);

	const isRunning = aiRunningThreadIds?.has(thread.id) ?? false;

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
			{sortedEntries.length > 0 && (
				<div className="comment-thread-items">
					{sortedEntries.map((entry, index) => (
						<EntryItem
							key={entry.id}
							entry={entry}
							threadId={thread.id}
							isFirstEntry={index === 0}
							severity={index === 0 ? thread.severity : undefined}
							thread={thread}
							onDelete={onDeleteThread}
							onUpdate={onUpdateEntry}
							onCopy={onCopyThread}
							onResolve={onResolveThread}
						/>
					))}
				</div>
			)}
			{isRunning && (
				<button
					type="button"
					className="comment-thread-ai-thinking"
					onClick={(e) => {
						e.stopPropagation();
						onOpenThreadAIModal?.(thread.id);
					}}
				>
					<Loader2 className="h-3 w-3 animate-spin" />
					AI is thinking...
				</button>
			)}
			{!isRunning && aiTaskThreadIds?.has(thread.id) && (
				<button
					type="button"
					className="comment-thread-ai-log"
					onClick={(e) => {
						e.stopPropagation();
						onOpenThreadAIModal?.(thread.id);
					}}
				>
					<ScrollText className="h-3 w-3" />
					View AI Log
				</button>
			)}
			<div className="comment-thread-reply">
				<textarea
					ref={textareaRef}
					className="comment-thread-textarea"
					placeholder="Type a reply..."
					rows={3}
				/>
				<div className="comment-thread-actions">
					<button
						type="button"
						className="comment-thread-cancel-btn"
						onClick={handleCancelClick}
					>
						Cancel
					</button>
					<button
						type="button"
						className="comment-thread-submit-btn"
						onClick={handleSend}
					>
						Send
					</button>
				</div>
			</div>
			{(onImplementThread || onPostToPr) && (
				<div className="comment-thread-conclusion">
					{onImplementThread && (
						<button
							type="button"
							className="comment-thread-conclusion-btn"
							onClick={(e) => {
								e.stopPropagation();
								onImplementThread(thread.id);
							}}
						>
							<Play className="h-3.5 w-3.5" />
							Implement
						</button>
					)}
					{onPostToPr && (
						<button
							type="button"
							className="comment-thread-conclusion-btn"
							onClick={(e) => {
								e.stopPropagation();
								onPostToPr(thread.id);
							}}
						>
							<MessageSquareShare className="h-3.5 w-3.5" />
							Post to PR
						</button>
					)}
				</div>
			)}
		</>
	);
}
