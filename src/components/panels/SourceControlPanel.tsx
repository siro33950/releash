import {
	ArrowDown,
	ArrowUp,
	ChevronDown,
	ChevronRight,
	Minus,
	Pencil,
	Plus,
	X,
} from "lucide-react";
import { useCallback, useState } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitStatus } from "@/hooks/useGitStatus";
import { cn } from "@/lib/utils";
import type { GitFileStatus } from "@/types/git";

function statusColor(status: string): string {
	switch (status) {
		case "new":
			return "text-status-untracked";
		case "modified":
			return "text-status-modified";
		case "deleted":
			return "text-status-deleted";
		case "renamed":
			return "text-status-modified";
		default:
			return "text-muted-foreground";
	}
}

function StatusIcon({ status }: { status: string }) {
	const color = statusColor(status);
	const iconClass = cn("h-3.5 w-3.5 shrink-0", color);
	switch (status) {
		case "modified":
		case "renamed":
			return <Pencil className={iconClass} />;
		case "new":
			return <Plus className={iconClass} />;
		case "deleted":
			return <Minus className={iconClass} />;
		default:
			return null;
	}
}

function formatPath(path: string): { dir: string; name: string } {
	const parts = path.split("/");
	const name = parts.pop() ?? path;
	const dir = parts.length > 0 ? `${parts.join("/")}/` : "";
	return { dir, name };
}

function FileStatusItem({
	entry,
	statusField,
	rootPath,
	onSelect,
	actionLabel,
	onAction,
}: {
	entry: GitFileStatus;
	statusField: "index_status" | "worktree_status";
	rootPath: string;
	onSelect?: (path: string) => void;
	actionLabel: string;
	onAction: () => void;
}) {
	const status = entry[statusField];
	const { dir, name } = formatPath(entry.path);

	return (
		<button
			type="button"
			className="group flex w-full items-center gap-1.5 px-4 py-1 text-sm hover:bg-sidebar-accent transition-colors"
			onClick={() => onSelect?.(`${rootPath}/${entry.path}`)}
		>
			<StatusIcon status={status} />
			<span className="truncate flex-1 text-left">
				<span className="text-muted-foreground">{dir}</span>
				<span className="font-semibold">{name}</span>
			</span>
			<button
				type="button"
				className="hidden group-hover:inline-flex items-center justify-center h-5 w-5 rounded hover:bg-sidebar-accent-foreground/10 shrink-0"
				onClick={(e) => {
					e.stopPropagation();
					onAction();
				}}
				title={actionLabel}
			>
				{statusField === "worktree_status" ? (
					<Plus className="h-3.5 w-3.5" />
				) : (
					<Minus className="h-3.5 w-3.5" />
				)}
			</button>
		</button>
	);
}

function CollapsibleSection({
	title,
	count,
	defaultOpen = true,
	actionLabel,
	actionIcon,
	onAction,
	children,
}: {
	title: string;
	count?: number;
	defaultOpen?: boolean;
	actionLabel?: string;
	actionIcon?: React.ReactNode;
	onAction?: () => void;
	children: React.ReactNode;
}) {
	const [open, setOpen] = useState(defaultOpen);

	return (
		<div className="overflow-hidden">
			<div className="flex items-center gap-1 px-2 py-1 text-xs font-semibold uppercase tracking-wide hover:bg-sidebar-accent transition-colors">
				<button
					type="button"
					className="flex flex-1 min-w-0 items-center gap-1"
					onClick={() => setOpen(!open)}
				>
					{open ? (
						<ChevronDown className="h-3.5 w-3.5 shrink-0" />
					) : (
						<ChevronRight className="h-3.5 w-3.5 shrink-0" />
					)}
					<span className="flex-1 text-left truncate">
						{title}
						{count != null ? ` (${count})` : ""}
					</span>
				</button>
				{actionLabel && onAction && (
					<button
						type="button"
						className="inline-flex items-center justify-center h-5 w-5 min-w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-accent-foreground/10 transition-colors shrink-0"
						onClick={onAction}
						title={actionLabel}
					>
						{actionIcon}
					</button>
				)}
			</div>
			{open && children}
		</div>
	);
}

export interface SourceControlPanelProps {
	rootPath: string | null;
	onSelectFile?: (path: string) => void;
}

export function SourceControlPanel({
	rootPath,
	onSelectFile,
}: SourceControlPanelProps) {
	const {
		stagedFiles,
		changedFiles,
		refresh: refreshStatus,
	} = useGitStatus(rootPath);
	const { stage, unstage, commit, push } = useGitActions();

	const [commitSummary, setCommitSummary] = useState("");
	const [commitDescription, setCommitDescription] = useState("");
	const [error, setError] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);

	const totalChanges = stagedFiles.length + changedFiles.length;

	const handleStage = useCallback(
		async (paths: string[]) => {
			if (!rootPath) return;
			try {
				setError(null);
				await stage(rootPath, paths);
				refreshStatus();
			} catch (e) {
				setError(String(e));
			}
		},
		[rootPath, stage, refreshStatus],
	);

	const handleUnstage = useCallback(
		async (paths: string[]) => {
			if (!rootPath) return;
			try {
				setError(null);
				await unstage(rootPath, paths);
				refreshStatus();
			} catch (e) {
				setError(String(e));
			}
		},
		[rootPath, unstage, refreshStatus],
	);

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
		} catch (e) {
			setError(String(e));
		} finally {
			setLoading(false);
		}
	}, [rootPath, commitSummary, commitDescription, commit, refreshStatus]);

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
			<div className="h-full flex items-center justify-center bg-sidebar">
				<span className="text-sm text-muted-foreground">No folder opened</span>
			</div>
		);
	}

	return (
		<div className="h-full flex flex-col bg-sidebar">
			{/* Header */}
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate">
					{totalChanges} file changes
				</span>
			</div>

			{/* File Lists */}
			<ScrollArea className="flex-1 min-h-0 [&>[data-slot=scroll-area-viewport]>div]:block!">
				<CollapsibleSection
					title="Unstaged Files"
					count={changedFiles.length}
					actionLabel="Stage All Changes"
					actionIcon={<ArrowDown className="h-3.5 w-3.5" />}
					onAction={() => handleStage([])}
				>
					{changedFiles.length === 0 && (
						<div className="px-4 py-1.5 text-xs text-muted-foreground">
							No unstaged changes
						</div>
					)}
					{changedFiles.map((entry) => (
						<FileStatusItem
							key={`changed-${entry.path}`}
							entry={entry}
							statusField="worktree_status"
							rootPath={rootPath}
							onSelect={onSelectFile}
							actionLabel="Stage"
							onAction={() => handleStage([entry.path])}
						/>
					))}
				</CollapsibleSection>

				<CollapsibleSection
					title="Staged Files"
					count={stagedFiles.length}
					actionLabel="Unstage All Changes"
					actionIcon={<ArrowUp className="h-3.5 w-3.5" />}
					onAction={() => handleUnstage([])}
				>
					{stagedFiles.length === 0 && (
						<div className="px-4 py-1.5 text-xs text-muted-foreground">
							No staged changes
						</div>
					)}
					{stagedFiles.map((entry) => (
						<FileStatusItem
							key={`staged-${entry.path}`}
							entry={entry}
							statusField="index_status"
							rootPath={rootPath}
							onSelect={onSelectFile}
							actionLabel="Unstage"
							onAction={() => handleUnstage([entry.path])}
						/>
					))}
				</CollapsibleSection>

				{totalChanges === 0 && (
					<div className="px-3 py-4 text-sm text-muted-foreground">
						No changes
					</div>
				)}
			</ScrollArea>

			{/* Commit Area (bottom fixed) */}
			<div className="border-t border-border px-3 py-2 shrink-0 flex flex-col gap-1.5">
				<div className="relative">
					<input
						type="text"
						className="w-full bg-transparent border border-border rounded px-2 py-1 text-xs outline-none focus:border-primary pr-8"
						placeholder="Commit summary"
						value={commitSummary}
						onChange={(e) => setCommitSummary(e.target.value)}
						onKeyDown={(e) => {
							if (e.key === "Enter" && !e.shiftKey) handleCommit();
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
						className="flex-1 flex items-center justify-center gap-1 bg-primary text-primary-foreground rounded px-2 py-1 text-xs font-medium hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
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
		</div>
	);
}
