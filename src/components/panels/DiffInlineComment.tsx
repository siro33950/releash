import {
	Bot,
	Check,
	CheckCircle2,
	MessageSquareReply,
	Send,
	Trash2,
	User,
	X,
} from "lucide-react";
import { useState } from "react";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { useReviewThreadHandoff } from "@/contexts/ReviewThreadHandoffContext";
import { formatRelativeTime } from "@/lib/formatRelativeTime";
import {
	getThreadEndLine,
	getThreadLineNumber,
	type ReviewDiscussionThread,
} from "@/types/diffComment";
import type { ReviewActorKind } from "@/types/protocol";
import { DiffCommentBody } from "./DiffCommentBody";

interface DiffInlineCommentProps {
	comment: ReviewDiscussionThread;
	onAppend?: (threadId: string, content: string) => Promise<void>;
	onResolve?: (
		threadId: string,
		outcome: string,
		summary: string,
	) => Promise<void>;
	onDelete?: (threadId: string) => Promise<void>;
}

function rangeLabel(comment: ReviewDiscussionThread): string | null {
	const lineNumber = getThreadLineNumber(comment);
	const endLine = getThreadEndLine(comment);
	if (lineNumber && endLine && endLine !== lineNumber) {
		return `L${lineNumber}-${endLine}`;
	}
	if (lineNumber) return `L${lineNumber}`;
	return null;
}

interface AuthorIconProps {
	kind: ReviewActorKind;
}

function AuthorIcon({ kind }: AuthorIconProps) {
	const Icon = kind === "agent" ? Bot : User;
	const label = kind === "agent" ? "Agent author" : "Human author";
	return (
		<div
			role="img"
			aria-label={label}
			data-testid={`author-icon-${kind}`}
			className="size-6 rounded-full bg-muted text-muted-foreground flex items-center justify-center shrink-0"
		>
			<Icon className="size-3.5" aria-hidden="true" />
		</div>
	);
}

export function DiffInlineComment({
	comment,
	onAppend,
	onResolve,
	onDelete,
}: DiffInlineCommentProps) {
	const [reply, setReply] = useState("");
	const [resolveSummary, setResolveSummary] = useState("");
	const [busy, setBusy] = useState(false);
	const [deleteOpen, setDeleteOpen] = useState(false);
	const label = rangeLabel(comment);
	const disabled = busy || comment.state === "resolved";
	// spec issues-1022 "Thread handoff contract": Diff Thread を active な
	// AgentChat session の入力として共有する。active session 不在時は disabled。
	const { canSend: canSendToAgent, sendThreadToAgent } =
		useReviewThreadHandoff();
	const sendToAgentTitle = canSendToAgent
		? "Diff Thread を現在の Agent に送信"
		: "アクティブな Agent セッションがありません";

	const run = async (action: () => Promise<void>) => {
		setBusy(true);
		try {
			await action();
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="flex flex-col gap-2 mx-2 my-1 p-3 rounded-md border border-border bg-card shadow-sm">
			<div className="flex items-center justify-between gap-2">
				{label ? (
					<span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground font-mono bg-muted px-1.5 py-0.5 rounded w-fit">
						{label}
					</span>
				) : (
					<span />
				)}
				<div className="flex items-center gap-1 shrink-0">
					{comment.state === "resolved" && (
						<span className="inline-flex items-center gap-1 text-[10px] bg-green-600/15 text-green-600 px-1.5 py-0.5 rounded-full shrink-0">
							<CheckCircle2 className="size-3" />
							resolved
						</span>
					)}
					<Button
						type="button"
						variant="ghost"
						size="icon-xs"
						title={sendToAgentTitle}
						aria-label={sendToAgentTitle}
						disabled={busy || !canSendToAgent}
						onClick={() =>
							run(async () => {
								await sendThreadToAgent(comment.id);
							})
						}
					>
						<Send className="size-3.5" />
					</Button>
					{onDelete && (
						<AlertDialog open={deleteOpen} onOpenChange={setDeleteOpen}>
							<AlertDialogTrigger asChild>
								<Button
									type="button"
									variant="ghost"
									size="icon-xs"
									title="Delete thread"
									aria-label="Delete thread"
									disabled={busy}
								>
									<Trash2 className="size-3.5" />
								</Button>
							</AlertDialogTrigger>
							<AlertDialogContent>
								<AlertDialogHeader>
									<AlertDialogTitle>Delete this thread?</AlertDialogTitle>
									<AlertDialogDescription>
										This will remove the thread from the review list. The
										deletion is recorded in the thread history and cannot be
										undone from the UI.
									</AlertDialogDescription>
								</AlertDialogHeader>
								<AlertDialogFooter>
									<AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
									<AlertDialogAction
										disabled={busy}
										onClick={(e) => {
											e.preventDefault();
											run(async () => {
												await onDelete(comment.id);
												setDeleteOpen(false);
											});
										}}
									>
										Delete
									</AlertDialogAction>
								</AlertDialogFooter>
							</AlertDialogContent>
						</AlertDialog>
					)}
				</div>
			</div>

			<div className="divide-y divide-border">
				{comment.comments.map((entry) => (
					<div
						key={entry.id}
						className="flex gap-2.5 py-2.5 first:pt-0 last:pb-0"
					>
						<AuthorIcon kind={entry.author.kind} />
						<div className="min-w-0 flex-1">
							<div className="flex flex-wrap items-baseline gap-1.5 text-xs">
								<span className="font-semibold text-foreground">
									{entry.author.displayName}
								</span>
								<span className="text-[10px] px-1 py-0.5 rounded bg-muted text-muted-foreground">
									{entry.author.kind}
								</span>
								<span
									className="text-muted-foreground"
									title={new Date(entry.createdAt).toLocaleString()}
								>
									{formatRelativeTime(entry.createdAt)}
								</span>
							</div>
							<DiffCommentBody content={entry.content} className="mt-1" />
						</div>
					</div>
				))}
			</div>

			{comment.resolve && (
				<div className="rounded border border-green-600/20 bg-green-600/10 px-2 py-1.5 text-[11px] text-green-700 dark:text-green-400">
					<div className="flex items-center gap-1 font-medium">
						<CheckCircle2 className="size-3" />
						{comment.resolve.outcome} by {comment.resolve.actor.kind} ·{" "}
						{comment.resolve.actor.displayName}
					</div>
					{comment.resolve.summary && (
						<DiffCommentBody
							content={comment.resolve.summary}
							className="mt-1 text-foreground"
						/>
					)}
				</div>
			)}

			{comment.state === "open" && (
				<div className="flex flex-col gap-1.5">
					<Textarea
						value={reply}
						onChange={(e) => setReply(e.target.value)}
						placeholder="Reply..."
						className="min-h-[64px] text-sm resize-none bg-background"
					/>
					<div className="flex justify-end">
						<Button
							type="button"
							variant="outline"
							size="sm"
							className="h-7 px-2 text-xs"
							disabled={disabled || reply.trim() === "" || !onAppend}
							onClick={() =>
								run(async () => {
									await onAppend?.(comment.id, reply.trim());
									setReply("");
								})
							}
						>
							<MessageSquareReply className="size-3" />
							Reply
						</Button>
					</div>
				</div>
			)}

			{comment.canResolve && comment.state === "open" && (
				<div className="flex items-center gap-1">
					<input
						value={resolveSummary}
						onChange={(e) => setResolveSummary(e.target.value)}
						placeholder="Resolution summary"
						className="min-w-0 flex-1 h-7 px-2 rounded border border-input bg-background text-xs"
					/>
					<Button
						type="button"
						variant="outline"
						size="icon-xs"
						disabled={disabled || resolveSummary.trim() === "" || !onResolve}
						title="Resolve"
						onClick={() =>
							run(async () => {
								await onResolve?.(
									comment.id,
									"resolved",
									resolveSummary.trim(),
								);
								setResolveSummary("");
							})
						}
					>
						<Check className="size-3.5" />
					</Button>
					<Button
						type="button"
						variant="ghost"
						size="icon-xs"
						title="Clear resolution summary"
						onClick={() => setResolveSummary("")}
					>
						<X className="size-3.5" />
					</Button>
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
