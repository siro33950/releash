import { ArrowUpFromLine, Check, Loader2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { FileStatusItem } from "@/components/panels/FileStatusItem";
import { CollapsibleSection } from "@/components/ui/collapsible-section";
import { InlineMessage } from "@/components/ui/inline-message";
import type { GitFileStatus } from "@/types/git";

interface RemoteSourceControlProps {
	stagedFiles: GitFileStatus[];
	changedFiles: GitFileStatus[];
	selectedPath: string | null;
	onSelectFile: (path: string) => void;
	onStage: (paths: string[]) => void;
	onUnstage: (paths: string[]) => void;
	onCommit: (message: string) => void;
	onPush: () => void;
	committing: boolean;
	pushing: boolean;
	pushResult: string | null;
	onClearPushResult: () => void;
	error: string | null;
	onClearError: () => void;
	onNavigateToDiff?: () => void;
	onRefresh?: () => void;
}

export function RemoteSourceControl({
	stagedFiles,
	changedFiles,
	selectedPath,
	onSelectFile,
	onStage,
	onUnstage,
	onCommit,
	onPush,
	committing,
	pushing,
	pushResult,
	onClearPushResult,
	error,
	onClearError,
	onNavigateToDiff,
	onRefresh,
}: RemoteSourceControlProps) {
	const totalChanges = stagedFiles.length + changedFiles.length;
	const [commitMessage, setCommitMessage] = useState("");

	const prevCommittingRef = useRef(false);

	useEffect(() => {
		if (prevCommittingRef.current && !committing && !error) {
			setCommitMessage("");
		}
		prevCommittingRef.current = committing;
	}, [committing, error]);

	const handleCommit = useCallback(() => {
		if (!commitMessage.trim() || committing) return;
		onCommit(commitMessage.trim());
	}, [commitMessage, committing, onCommit]);

	const handleSelectFile = useCallback(
		(path: string) => {
			onSelectFile(path);
			onNavigateToDiff?.();
		},
		[onSelectFile, onNavigateToDiff],
	);

	const handleStageAll = useCallback(() => onStage([]), [onStage]);
	const handleUnstageAll = useCallback(() => onUnstage([]), [onUnstage]);

	return (
		<div className="h-full flex flex-col bg-card">
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate text-muted-foreground flex-1">
					{totalChanges} file changes
				</span>
				{onRefresh && (
					<button
						type="button"
						className="inline-flex items-center justify-center h-5 w-5 rounded text-muted-foreground hover:text-foreground hover:bg-muted transition-colors shrink-0"
						onClick={onRefresh}
						title="Refresh"
					>
						<RefreshCw className="h-3.5 w-3.5" />
					</button>
				)}
			</div>

			<div className="flex-1 overflow-y-auto" style={{ minHeight: 0 }}>
				<CollapsibleSection title="Unstaged" count={changedFiles.length}>
					{changedFiles.length === 0 && (
						<div className="px-4 py-1.5 text-xs text-muted-foreground">
							No unstaged changes
						</div>
					)}
					{changedFiles.length > 0 && (
						<div className="flex justify-end px-2 py-0.5">
							<button
								type="button"
								className="text-[10px] text-muted-foreground hover:text-foreground transition-colors"
								onClick={handleStageAll}
							>
								Stage All
							</button>
						</div>
					)}
					{changedFiles.map((entry) => (
						<FileStatusItem
							key={`changed-${entry.path}`}
							entry={entry}
							statusField="worktree_status"
							selected={selectedPath === entry.path}
							onSelect={(e) => handleSelectFile(e.path)}
							actionLabel="Stage"
							onAction={() => onStage([entry.path])}
							alwaysShowAction
						/>
					))}
				</CollapsibleSection>

				<CollapsibleSection title="Staged" count={stagedFiles.length}>
					{stagedFiles.length === 0 && (
						<div className="px-4 py-1.5 text-xs text-muted-foreground">
							No staged changes
						</div>
					)}
					{stagedFiles.length > 0 && (
						<div className="flex justify-end px-2 py-0.5">
							<button
								type="button"
								className="text-[10px] text-muted-foreground hover:text-foreground transition-colors"
								onClick={handleUnstageAll}
							>
								Unstage All
							</button>
						</div>
					)}
					{stagedFiles.map((entry) => (
						<FileStatusItem
							key={`staged-${entry.path}`}
							entry={entry}
							statusField="index_status"
							selected={selectedPath === entry.path}
							onSelect={(e) => handleSelectFile(e.path)}
							actionLabel="Unstage"
							onAction={() => onUnstage([entry.path])}
							alwaysShowAction
						/>
					))}
				</CollapsibleSection>

				{totalChanges === 0 && (
					<div className="px-3 py-4 text-sm text-muted-foreground">
						No changes
					</div>
				)}
			</div>

			<div className="px-3 py-2 border-t border-border shrink-0">
				<textarea
					value={commitMessage}
					onChange={(e) => setCommitMessage(e.target.value)}
					placeholder="Commit message..."
					rows={2}
					className="w-full px-2 py-1.5 text-sm bg-input text-foreground border border-border rounded resize-none outline-none focus:border-primary placeholder:text-muted-foreground"
				/>
				<div className="flex gap-2 mt-1.5">
					<button
						type="button"
						disabled={
							!commitMessage.trim() || stagedFiles.length === 0 || committing
						}
						className="flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded bg-success hover:bg-success/90 text-success-foreground transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
						onClick={handleCommit}
					>
						{committing ? (
							<Loader2 className="h-3.5 w-3.5 animate-spin" />
						) : (
							<Check className="h-3.5 w-3.5" />
						)}
						{committing ? "Committing..." : "Commit"}
					</button>
					<button
						type="button"
						disabled={pushing}
						className="flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded bg-primary hover:bg-primary/90 text-primary-foreground transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
						onClick={onPush}
					>
						{pushing ? (
							<Loader2 className="h-3.5 w-3.5 animate-spin" />
						) : (
							<ArrowUpFromLine className="h-3.5 w-3.5" />
						)}
						{pushing ? "Pushing..." : "Push"}
					</button>
				</div>
			</div>

			{pushResult && (
				<InlineMessage
					type="success"
					className="px-3 py-2 border-t border-border"
					onDismiss={onClearPushResult}
				>
					{pushResult}
				</InlineMessage>
			)}

			{error && (
				<InlineMessage
					className="px-3 py-2 border-t border-border"
					onDismiss={onClearError}
				>
					{error}
				</InlineMessage>
			)}
		</div>
	);
}
