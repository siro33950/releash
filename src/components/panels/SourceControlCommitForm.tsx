import { ArrowUp, Loader2 } from "lucide-react";
import { useEffect } from "react";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Input } from "@/components/ui/input";
import { Message } from "@/components/ui/message";
import { cn } from "@/lib/utils";

interface CommitFormProps {
	commitSummary: string;
	commitDescription: string;
	loading: boolean;
	pushing: boolean;
	error: string | null;
	successMessage: string | null;
	stagedFilesCount: number;
	ahead: number;
	behind: number;
	hasUpstream: boolean;
	onSummaryChange: (value: string) => void;
	onDescriptionChange: (value: string) => void;
	onCommit: () => void;
	onPush: () => void;
	onDismissError: () => void;
	onDismissSuccess: () => void;
}

export function CommitForm({
	commitSummary,
	commitDescription,
	loading,
	pushing,
	error,
	successMessage,
	stagedFilesCount,
	ahead,
	behind,
	hasUpstream,
	onSummaryChange,
	onDescriptionChange,
	onCommit,
	onPush,
	onDismissError,
	onDismissSuccess,
}: CommitFormProps) {
	useEffect(() => {
		if (!successMessage) return;
		const timer = setTimeout(onDismissSuccess, 3000);
		return () => clearTimeout(timer);
	}, [successMessage, onDismissSuccess]);
	return (
		<div className="border-t border-border px-3 py-2 shrink-0 flex flex-col gap-1.5">
			<div className="relative">
				<Input
					type="text"
					variant="panel"
					size="sm"
					className="pr-8"
					placeholder="Commit summary"
					value={commitSummary}
					onChange={(e) => onSummaryChange(e.target.value)}
					onKeyDown={(e) => {
						if (
							e.key === "Enter" &&
							!e.shiftKey &&
							commitSummary.trim() &&
							stagedFilesCount > 0 &&
							!loading
						)
							onCommit();
					}}
				/>
				<span
					className={cn(
						"absolute right-2 top-1/2 -translate-y-1/2 text-[10px] font-mono",
						commitSummary.length > 72
							? "text-destructive"
							: "text-muted-foreground",
					)}
				>
					{commitSummary.length}
				</span>
			</div>
			<textarea
				className="w-full bg-transparent border border-border rounded px-2 py-1 text-xs outline-none focus:border-primary resize-y min-h-[40px]"
				placeholder="Description"
				value={commitDescription}
				onChange={(e) => onDescriptionChange(e.target.value)}
				rows={2}
			/>
			<div className="flex items-center gap-1.5">
				<button
					type="button"
					className="flex-1 flex items-center justify-center gap-1 bg-accent text-accent-foreground rounded px-2 py-1 text-xs font-medium hover:bg-accent/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
					disabled={!commitSummary.trim() || stagedFilesCount === 0 || loading}
					onClick={onCommit}
				>
					Commit
				</button>
				<button
					type="button"
					className="flex items-center justify-center gap-1 border border-border rounded px-2 py-1 text-xs font-medium hover:bg-sidebar-accent transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
					disabled={loading}
					onClick={onPush}
				>
					{pushing ? (
						<>
							<Loader2 className="h-3 w-3 animate-spin" />
							Pushing...
						</>
					) : (
						<>
							<ArrowUp className="h-3 w-3" />
							Push
						</>
					)}
				</button>
				{hasUpstream && (ahead > 0 || behind > 0) && (
					<span className="shrink-0 text-[10px] text-muted-foreground">
						{ahead > 0 && `↑${ahead}`}
						{ahead > 0 && behind > 0 && " "}
						{behind > 0 && `↓${behind}`}
					</span>
				)}
			</div>
			{successMessage && (
				<Message
					message={successMessage}
					severity="success"
					onDismiss={onDismissSuccess}
				/>
			)}
			{error && <Message message={error} onDismiss={onDismissError} />}
		</div>
	);
}

interface DiscardConfirmDialogProps {
	target: { path: string; paths: string[] } | null;
	onConfirm: () => void;
	onCancel: () => void;
}

export function DiscardConfirmDialog({
	target,
	onConfirm,
	onCancel,
}: DiscardConfirmDialogProps) {
	return (
		<AlertDialog open={target !== null} onOpenChange={(o) => !o && onCancel()}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>Discard Changes</AlertDialogTitle>
					<AlertDialogDescription>
						Discard changes in "{target?.path}"? This action cannot be undone.
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={onCancel}>Cancel</AlertDialogCancel>
					<AlertDialogAction onClick={onConfirm}>Discard</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
