import { Check, CheckCircle2, MessageSquareReply, X } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import {
	getThreadEndLine,
	getThreadLineNumber,
	type ReviewDiscussionThread,
} from "@/types/diffComment";
import type { ReviewStanceValue } from "@/types/protocol";

interface DiffInlineCommentProps {
	comment: ReviewDiscussionThread;
	onAppend?: (threadId: string, content: string) => Promise<void>;
	onSetStance?: (threadId: string, value: ReviewStanceValue) => Promise<void>;
	onResolve?: (
		threadId: string,
		outcome: string,
		summary: string,
	) => Promise<void>;
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

export function DiffInlineComment({
	comment,
	onAppend,
	onSetStance,
	onResolve,
}: DiffInlineCommentProps) {
	const [reply, setReply] = useState("");
	const [resolveSummary, setResolveSummary] = useState("");
	const [busy, setBusy] = useState(false);
	const label = rangeLabel(comment);
	const currentStance = comment.myStance;
	const disabled = busy || comment.state === "resolved";

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
				{label && (
					<span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground font-mono bg-muted px-1.5 py-0.5 rounded w-fit">
						{label}
					</span>
				)}
				{comment.state === "resolved" && (
					<span className="inline-flex items-center gap-1 text-[10px] bg-green-600/15 text-green-600 px-1.5 py-0.5 rounded-full shrink-0">
						<CheckCircle2 className="size-3" />
						resolved
					</span>
				)}
			</div>

			<div className="space-y-2">
				{comment.comments.map((entry) => (
					<div key={entry.id} className="text-sm">
						<div className="text-[10px] text-muted-foreground">
							{entry.author.kind} · {entry.author.displayName}
						</div>
						<p className="whitespace-pre-wrap break-words leading-relaxed">
							{entry.content}
						</p>
					</div>
				))}
			</div>

			<div className="flex flex-wrap gap-1">
				{comment.stances.map((stance) => (
					<span
						key={`${stance.actor.kind}:${stance.actor.displayName}`}
						className="inline-flex items-center gap-1 rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground"
					>
						<span>{stance.actor.kind}</span>
						<span>{stance.actor.displayName}</span>
						<span className="text-foreground">{stance.value}</span>
					</span>
				))}
			</div>

			{comment.resolve && (
				<div className="rounded border border-green-600/20 bg-green-600/10 px-2 py-1 text-[11px] text-green-700 dark:text-green-400">
					<div>
						{comment.resolve.outcome} by {comment.resolve.actor.kind} ·{" "}
						{comment.resolve.actor.displayName}
					</div>
					<div className="break-words">{comment.resolve.summary}</div>
				</div>
			)}

			<div className="flex flex-wrap items-center gap-1">
				{(["agree", "disagree", "none"] as const).map((value) => (
					<Button
						key={value}
						type="button"
						variant={currentStance === value ? "default" : "outline"}
						size="sm"
						className="h-7 px-2 text-xs"
						disabled={disabled || !onSetStance}
						onClick={() =>
							run(() => onSetStance?.(comment.id, value) ?? Promise.resolve())
						}
					>
						{value}
					</Button>
				))}
			</div>

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
