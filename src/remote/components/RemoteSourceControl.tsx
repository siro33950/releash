import { ArrowUpFromLine, Check, Loader2, RefreshCw, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { FileStatusItem } from "@/components/panels/FileStatusItem";
import { CollapsibleSection } from "@/components/ui/collapsible-section";
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
		<div className="h-full flex flex-col bg-neutral-900">
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-neutral-800 shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate text-neutral-400 flex-1">
					{totalChanges} file changes
				</span>
				{onRefresh && (
					<button
						type="button"
						className="inline-flex items-center justify-center h-5 w-5 rounded text-neutral-400 hover:text-neutral-200 hover:bg-neutral-700 transition-colors shrink-0"
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
						<div className="px-4 py-1.5 text-xs text-neutral-500">
							No unstaged changes
						</div>
					)}
					{changedFiles.length > 0 && (
						<div className="flex justify-end px-2 py-0.5">
							<button
								type="button"
								className="text-[10px] text-neutral-400 hover:text-neutral-200 transition-colors"
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
						<div className="px-4 py-1.5 text-xs text-neutral-500">
							No staged changes
						</div>
					)}
					{stagedFiles.length > 0 && (
						<div className="flex justify-end px-2 py-0.5">
							<button
								type="button"
								className="text-[10px] text-neutral-400 hover:text-neutral-200 transition-colors"
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
					<div className="px-3 py-4 text-sm text-neutral-500">No changes</div>
				)}
			</div>

			<div className="px-3 py-2 border-t border-neutral-800 shrink-0">
				<textarea
					value={commitMessage}
					onChange={(e) => setCommitMessage(e.target.value)}
					placeholder="Commit message..."
					rows={2}
					className="w-full px-2 py-1.5 text-sm bg-neutral-800 text-neutral-200 border border-neutral-700 rounded resize-none outline-none focus:border-blue-500 placeholder:text-neutral-500"
				/>
				<div className="flex gap-2 mt-1.5">
					<button
						type="button"
						disabled={
							!commitMessage.trim() || stagedFiles.length === 0 || committing
						}
						className="flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded bg-green-700 hover:bg-green-600 text-green-50 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
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
						className="flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded bg-blue-700 hover:bg-blue-600 text-blue-50 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
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
				<div className="flex items-start gap-1 px-3 py-2 text-green-400 text-xs border-t border-neutral-800">
					<span className="flex-1 break-all">{pushResult}</span>
					<button
						type="button"
						className="shrink-0"
						aria-label="閉じる"
						onClick={onClearPushResult}
					>
						<X className="h-3 w-3" />
					</button>
				</div>
			)}

			{error && (
				<div className="flex items-start gap-1 px-3 py-2 text-red-400 text-xs border-t border-neutral-800">
					<span className="flex-1 break-all">{error}</span>
					<button
						type="button"
						className="shrink-0"
						aria-label="エラーを閉じる"
						onClick={onClearError}
					>
						<X className="h-3 w-3" />
					</button>
				</div>
			)}
		</div>
	);
}
