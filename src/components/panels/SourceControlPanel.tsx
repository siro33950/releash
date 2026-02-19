import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { ArrowDown, ArrowUp, RefreshCw, X } from "lucide-react";
import { useCallback, useState } from "react";
import { FileStatusItem } from "@/components/panels/FileStatusItem";
import { SourceControlContextMenu } from "@/components/panels/SourceControlContextMenu";
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
import { CollapsibleSection } from "@/components/ui/collapsible-section";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitStatus } from "@/hooks/useGitStatus";
import { cn } from "@/lib/utils";
import { EmptyState } from "./EmptyState";

export interface SourceControlPanelProps {
	rootPath: string | null;
	onSelectFile?: (path: string) => void;
	onGitChanged?: () => void;
	gitRefreshKey?: number;
}

export function SourceControlPanel({
	rootPath,
	onSelectFile,
	onGitChanged,
	gitRefreshKey,
}: SourceControlPanelProps) {
	const {
		stagedFiles,
		changedFiles,
		refresh: refreshStatus,
	} = useGitStatus(rootPath, gitRefreshKey);
	const { stage, unstage, discard, commit, push } = useGitActions();

	const [commitSummary, setCommitSummary] = useState("");
	const [commitDescription, setCommitDescription] = useState("");
	const [error, setError] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);
	const [discardTarget, setDiscardTarget] = useState<{
		path: string;
		paths: string[];
	} | null>(null);

	const totalChanges = stagedFiles.length + changedFiles.length;

	const handleStage = useCallback(
		async (paths: string[]) => {
			if (!rootPath) return;
			try {
				setError(null);
				await stage(rootPath, paths);
				refreshStatus();
				onGitChanged?.();
			} catch (e) {
				setError(String(e));
			}
		},
		[rootPath, stage, refreshStatus, onGitChanged],
	);

	const handleUnstage = useCallback(
		async (paths: string[]) => {
			if (!rootPath) return;
			try {
				setError(null);
				await unstage(rootPath, paths);
				refreshStatus();
				onGitChanged?.();
			} catch (e) {
				setError(String(e));
			}
		},
		[rootPath, unstage, refreshStatus, onGitChanged],
	);

	const handleDiscard = useCallback(async () => {
		if (!rootPath || !discardTarget) return;
		try {
			setError(null);
			await discard(rootPath, discardTarget.paths);
			refreshStatus();
			onGitChanged?.();
		} catch (e) {
			setError(String(e));
		} finally {
			setDiscardTarget(null);
		}
	}, [rootPath, discardTarget, discard, refreshStatus, onGitChanged]);

	const handleCommit = useCallback(async () => {
		if (!rootPath || !commitSummary.trim()) return;
		const message = commitDescription.trim()
			? `${commitSummary}\n\n${commitDescription}`
			: commitSummary;
		try {
			setError(null);
			setLoading(true);
			await commit(rootPath, message);
			setCommitSummary("");
			setCommitDescription("");
			refreshStatus();
			onGitChanged?.();
		} catch (e) {
			setError(String(e));
		} finally {
			setLoading(false);
		}
	}, [
		rootPath,
		commitSummary,
		commitDescription,
		commit,
		refreshStatus,
		onGitChanged,
	]);

	const handlePush = useCallback(async () => {
		if (!rootPath) return;
		try {
			setError(null);
			setLoading(true);
			await push(rootPath);
		} catch (e) {
			setError(String(e));
		} finally {
			setLoading(false);
		}
	}, [rootPath, push]);

	if (!rootPath) {
		return (
			<div className="h-full bg-sidebar">
				<EmptyState title="No folder opened" />
			</div>
		);
	}

	return (
		<div className="h-full flex flex-col bg-sidebar">
			{/* Header */}
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate flex-1">
					{totalChanges} file changes
				</span>
				<button
					type="button"
					className="inline-flex items-center justify-center h-5 w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-accent-foreground/10 transition-colors shrink-0"
					onClick={refreshStatus}
					title="Refresh"
				>
					<RefreshCw className="h-3.5 w-3.5" />
				</button>
			</div>

			{/* File Lists */}
			<ScrollArea className="flex-1 min-h-0 [&>[data-slot=scroll-area-viewport]>div]:block!">
				<CollapsibleSection
					title="Unstaged Files"
					count={changedFiles.length}
					headerClassName="gap-1 px-2 py-1 font-semibold uppercase tracking-wide"
					chevronClassName="h-3.5 w-3.5"
					actions={
						<button
							type="button"
							className="inline-flex items-center justify-center h-5 w-5 min-w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-accent-foreground/10 transition-colors shrink-0"
							onClick={() => handleStage([])}
							title="Stage All Changes"
						>
							<ArrowDown className="h-3.5 w-3.5" />
						</button>
					}
				>
					{changedFiles.length === 0 && (
						<EmptyState
							compact
							title="No unstaged changes"
							className="px-4 py-1.5"
						/>
					)}
					{changedFiles.map((entry) => (
						<SourceControlContextMenu
							key={`changed-${entry.path}`}
							variant="unstaged"
							onOpenChanges={() => onSelectFile?.(`${rootPath}/${entry.path}`)}
							onStage={() => handleStage([entry.path])}
							onDiscard={() =>
								setDiscardTarget({
									path: entry.path,
									paths: [entry.path],
								})
							}
							onCopyPath={() =>
								navigator.clipboard.writeText(`${rootPath}/${entry.path}`)
							}
							onCopyRelativePath={() =>
								navigator.clipboard.writeText(entry.path)
							}
							onRevealInFinder={() =>
								revealItemInDir(`${rootPath}/${entry.path}`)
							}
						>
							<FileStatusItem
								entry={entry}
								statusField="worktree_status"
								onSelect={(e) => onSelectFile?.(`${rootPath}/${e.path}`)}
								actionLabel="Stage"
								onAction={() => handleStage([entry.path])}
							/>
						</SourceControlContextMenu>
					))}
				</CollapsibleSection>

				<CollapsibleSection
					title="Staged Files"
					count={stagedFiles.length}
					headerClassName="gap-1 px-2 py-1 font-semibold uppercase tracking-wide"
					chevronClassName="h-3.5 w-3.5"
					actions={
						<button
							type="button"
							className="inline-flex items-center justify-center h-5 w-5 min-w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-accent-foreground/10 transition-colors shrink-0"
							onClick={() => handleUnstage([])}
							title="Unstage All Changes"
						>
							<ArrowUp className="h-3.5 w-3.5" />
						</button>
					}
				>
					{stagedFiles.length === 0 && (
						<EmptyState
							compact
							title="No staged changes"
							className="px-4 py-1.5"
						/>
					)}
					{stagedFiles.map((entry) => (
						<SourceControlContextMenu
							key={`staged-${entry.path}`}
							variant="staged"
							onOpenChanges={() => onSelectFile?.(`${rootPath}/${entry.path}`)}
							onUnstage={() => handleUnstage([entry.path])}
							onCopyPath={() =>
								navigator.clipboard.writeText(`${rootPath}/${entry.path}`)
							}
							onCopyRelativePath={() =>
								navigator.clipboard.writeText(entry.path)
							}
							onRevealInFinder={() =>
								revealItemInDir(`${rootPath}/${entry.path}`)
							}
						>
							<FileStatusItem
								entry={entry}
								statusField="index_status"
								onSelect={(e) => onSelectFile?.(`${rootPath}/${e.path}`)}
								actionLabel="Unstage"
								onAction={() => handleUnstage([entry.path])}
							/>
						</SourceControlContextMenu>
					))}
				</CollapsibleSection>

				{totalChanges === 0 && (
					<EmptyState
						compact
						title="No changes"
						className="px-3 py-4 text-sm"
					/>
				)}
			</ScrollArea>

			{/* Commit Area (bottom fixed) */}
			<div className="border-t border-border px-3 py-2 shrink-0 flex flex-col gap-1.5">
				<div className="relative">
					<Input
						type="text"
						variant="panel"
						size="sm"
						className="pr-8"
						placeholder="Commit summary"
						value={commitSummary}
						onChange={(e) => setCommitSummary(e.target.value)}
						onKeyDown={(e) => {
							if (
								e.key === "Enter" &&
								!e.shiftKey &&
								stagedFiles.length > 0 &&
								!loading
							)
								handleCommit();
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
					onChange={(e) => setCommitDescription(e.target.value)}
					rows={2}
				/>
				<div className="flex gap-1.5">
					<button
						type="button"
						className="flex-1 flex items-center justify-center gap-1 bg-accent text-accent-foreground rounded px-2 py-1 text-xs font-medium hover:bg-accent/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
						disabled={
							!commitSummary.trim() || stagedFiles.length === 0 || loading
						}
						onClick={handleCommit}
					>
						Commit
					</button>
					<button
						type="button"
						className="flex items-center justify-center gap-1 border border-border rounded px-2 py-1 text-xs font-medium hover:bg-sidebar-accent transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
						disabled={loading}
						onClick={handlePush}
					>
						Push
						<ArrowUp className="h-3 w-3" />
					</button>
				</div>
				{error && (
					<div className="flex items-start gap-1 text-destructive text-xs">
						<span className="flex-1 break-all">{error}</span>
						<button
							type="button"
							className="shrink-0"
							onClick={() => setError(null)}
						>
							<X className="h-3 w-3" />
						</button>
					</div>
				)}
			</div>

			{/* Discard Confirm Dialog */}
			<AlertDialog
				open={discardTarget !== null}
				onOpenChange={(o) => !o && setDiscardTarget(null)}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>変更の破棄</AlertDialogTitle>
						<AlertDialogDescription>
							「{discardTarget?.path}
							」の変更を破棄しますか？この操作は取り消せません。
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel onClick={() => setDiscardTarget(null)}>
							キャンセル
						</AlertDialogCancel>
						<AlertDialogAction onClick={handleDiscard}>破棄</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}
