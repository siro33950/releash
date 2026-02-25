import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { ArrowDown, ArrowUp, RefreshCw } from "lucide-react";
import { useCallback, useReducer } from "react";
import { FileStatusItem } from "@/components/panels/FileStatusItem";
import { SourceControlContextMenu } from "@/components/panels/SourceControlContextMenu";
import { CollapsibleSection } from "@/components/ui/collapsible-section";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useAheadBehind } from "@/hooks/useAheadBehind";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitStatus } from "@/hooks/useGitStatus";
import { formatGitError } from "@/lib/errorHandler";
import { EmptyState } from "./EmptyState";
import { CommitForm, DiscardConfirmDialog } from "./SourceControlCommitForm";

interface CommitFormState {
	summary: string;
	description: string;
	error: string | null;
	loading: boolean;
	pushing: boolean;
	successMessage: string | null;
	discardTarget: { path: string; paths: string[] } | null;
}

type CommitFormAction =
	| { type: "SET_SUMMARY"; value: string }
	| { type: "SET_DESCRIPTION"; value: string }
	| { type: "SET_ERROR"; error: string | null }
	| { type: "COMMIT_START" }
	| { type: "COMMIT_SUCCESS" }
	| { type: "COMMIT_ERROR"; error: string }
	| { type: "PUSH_START" }
	| { type: "PUSH_END" }
	| { type: "PUSH_ERROR"; error: string }
	| { type: "DISMISS_SUCCESS" }
	| { type: "SET_DISCARD_TARGET"; target: CommitFormState["discardTarget"] }
	| { type: "CLEAR_DISCARD" };

const initialCommitForm: CommitFormState = {
	summary: "",
	description: "",
	error: null,
	loading: false,
	pushing: false,
	successMessage: null,
	discardTarget: null,
};

export function commitFormReducer(
	state: CommitFormState,
	action: CommitFormAction,
): CommitFormState {
	switch (action.type) {
		case "SET_SUMMARY":
			return { ...state, summary: action.value };
		case "SET_DESCRIPTION":
			return { ...state, description: action.value };
		case "SET_ERROR":
			return { ...state, error: action.error };
		case "COMMIT_START":
			return { ...state, loading: true, error: null, successMessage: null };
		case "COMMIT_SUCCESS":
			return { ...state, loading: false, summary: "", description: "" };
		case "COMMIT_ERROR":
			return { ...state, loading: false, error: action.error };
		case "PUSH_START":
			return {
				...state,
				loading: true,
				pushing: true,
				error: null,
				successMessage: null,
			};
		case "PUSH_END":
			return {
				...state,
				loading: false,
				pushing: false,
				successMessage: "Pushed successfully",
			};
		case "PUSH_ERROR":
			return { ...state, loading: false, pushing: false, error: action.error };
		case "DISMISS_SUCCESS":
			return { ...state, successMessage: null };
		case "SET_DISCARD_TARGET":
			return { ...state, discardTarget: action.target };
		case "CLEAR_DISCARD":
			return { ...state, discardTarget: null };
	}
}

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
	const aheadBehind = useAheadBehind(rootPath, gitRefreshKey);

	const [form, dispatch] = useReducer(commitFormReducer, initialCommitForm);
	const {
		summary: commitSummary,
		description: commitDescription,
		error,
		loading,
		pushing,
		successMessage,
		discardTarget,
	} = form;

	const totalChanges = stagedFiles.length + changedFiles.length;

	const handleStage = useCallback(
		async (paths: string[]) => {
			if (!rootPath) return;
			try {
				dispatch({ type: "SET_ERROR", error: null });
				await stage(rootPath, paths);
				refreshStatus();
				onGitChanged?.();
			} catch (e) {
				dispatch({ type: "SET_ERROR", error: formatGitError(e) });
			}
		},
		[rootPath, stage, refreshStatus, onGitChanged],
	);

	const handleUnstage = useCallback(
		async (paths: string[]) => {
			if (!rootPath) return;
			try {
				dispatch({ type: "SET_ERROR", error: null });
				await unstage(rootPath, paths);
				refreshStatus();
				onGitChanged?.();
			} catch (e) {
				dispatch({ type: "SET_ERROR", error: formatGitError(e) });
			}
		},
		[rootPath, unstage, refreshStatus, onGitChanged],
	);

	const handleDiscard = useCallback(async () => {
		if (!rootPath || !discardTarget) return;
		try {
			dispatch({ type: "SET_ERROR", error: null });
			await discard(rootPath, discardTarget.paths);
			refreshStatus();
			onGitChanged?.();
		} catch (e) {
			dispatch({ type: "SET_ERROR", error: formatGitError(e) });
		} finally {
			dispatch({ type: "CLEAR_DISCARD" });
		}
	}, [rootPath, discardTarget, discard, refreshStatus, onGitChanged]);

	const handleCommit = useCallback(async () => {
		if (!rootPath || !commitSummary.trim()) return;
		const message = commitDescription.trim()
			? `${commitSummary}\n\n${commitDescription}`
			: commitSummary;
		try {
			dispatch({ type: "COMMIT_START" });
			await commit(rootPath, message);
			dispatch({ type: "COMMIT_SUCCESS" });
			refreshStatus();
			onGitChanged?.();
		} catch (e) {
			dispatch({ type: "COMMIT_ERROR", error: formatGitError(e) });
		}
	}, [
		rootPath,
		commitSummary,
		commitDescription,
		commit,
		refreshStatus,
		onGitChanged,
	]);

	const handleDismissSuccess = useCallback(() => {
		dispatch({ type: "DISMISS_SUCCESS" });
	}, []);

	const handlePush = useCallback(async () => {
		if (!rootPath) return;
		try {
			dispatch({ type: "PUSH_START" });
			await push(rootPath);
			dispatch({ type: "PUSH_END" });
			refreshStatus();
			onGitChanged?.();
		} catch (e) {
			dispatch({ type: "PUSH_ERROR", error: formatGitError(e) });
		}
	}, [rootPath, push, refreshStatus, onGitChanged]);

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
					className="inline-flex items-center justify-center h-5 w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-secondary-foreground/10 transition-colors shrink-0"
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
							className="inline-flex items-center justify-center h-5 w-5 min-w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-secondary-foreground/10 transition-colors shrink-0"
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
								dispatch({
									type: "SET_DISCARD_TARGET",
									target: {
										path: entry.path,
										paths: [entry.path],
									},
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
							className="inline-flex items-center justify-center h-5 w-5 min-w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-secondary-foreground/10 transition-colors shrink-0"
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
			<CommitForm
				commitSummary={commitSummary}
				commitDescription={commitDescription}
				loading={loading}
				pushing={pushing}
				error={error}
				successMessage={successMessage}
				stagedFilesCount={stagedFiles.length}
				ahead={aheadBehind?.ahead ?? 0}
				behind={aheadBehind?.behind ?? 0}
				hasUpstream={aheadBehind?.has_upstream ?? false}
				onSummaryChange={(value) => dispatch({ type: "SET_SUMMARY", value })}
				onDescriptionChange={(value) =>
					dispatch({ type: "SET_DESCRIPTION", value })
				}
				onCommit={handleCommit}
				onPush={handlePush}
				onDismissError={() => dispatch({ type: "SET_ERROR", error: null })}
				onDismissSuccess={handleDismissSuccess}
			/>

			{/* Discard Confirm Dialog */}
			<DiscardConfirmDialog
				target={discardTarget}
				onConfirm={handleDiscard}
				onCancel={() => dispatch({ type: "CLEAR_DISCARD" })}
			/>
		</div>
	);
}
