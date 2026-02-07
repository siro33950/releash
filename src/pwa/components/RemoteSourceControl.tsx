import {
	ChevronDown,
	ChevronRight,
	Minus,
	Pencil,
	Plus,
	RefreshCw,
	X,
} from "lucide-react";
import { useCallback, useState } from "react";
import type { GitFileStatus } from "@/types/git";

function statusColor(status: string): string {
	switch (status) {
		case "new":
			return "text-green-400";
		case "modified":
			return "text-yellow-400";
		case "deleted":
			return "text-red-400";
		case "renamed":
			return "text-yellow-400";
		default:
			return "text-neutral-500";
	}
}

function StatusIcon({ status }: { status: string }) {
	const color = statusColor(status);
	const iconClass = `h-3.5 w-3.5 shrink-0 ${color}`;
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
	selectedPath,
	onSelect,
	actionLabel,
	onAction,
}: {
	entry: GitFileStatus;
	statusField: "index_status" | "worktree_status";
	selectedPath: string | null;
	onSelect?: (path: string) => void;
	actionLabel: string;
	onAction: () => void;
}) {
	const status = entry[statusField];
	const { dir, name } = formatPath(entry.path);
	const isSelected = selectedPath === entry.path;

	return (
		// biome-ignore lint/a11y/useSemanticElements: ネストされたbuttonがあるためdiv+role使用
		<div
			role="button"
			tabIndex={0}
			className={`group flex w-full items-center gap-1.5 px-4 py-1 text-sm transition-colors cursor-pointer ${
				isSelected ? "bg-neutral-700" : "hover:bg-neutral-800"
			}`}
			onClick={() => onSelect?.(entry.path)}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					onSelect?.(entry.path);
				}
			}}
		>
			<StatusIcon status={status} />
			<span className="truncate flex-1 text-left">
				<span className="text-neutral-500">{dir}</span>
				<span className="font-semibold">{name}</span>
			</span>
			<button
				type="button"
				className="hidden group-hover:inline-flex items-center justify-center h-5 w-5 rounded hover:bg-neutral-600 shrink-0"
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
		</div>
	);
}

function CollapsibleSection({
	title,
	count,
	defaultOpen = true,
	children,
}: {
	title: string;
	count?: number;
	defaultOpen?: boolean;
	children: React.ReactNode;
}) {
	const [open, setOpen] = useState(defaultOpen);

	return (
		<div className="overflow-hidden">
			<button
				type="button"
				className="flex w-full items-center gap-1 px-2 py-1 text-xs font-semibold uppercase tracking-wide hover:bg-neutral-800 transition-colors"
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
			{open && children}
		</div>
	);
}

interface RemoteSourceControlProps {
	stagedFiles: GitFileStatus[];
	changedFiles: GitFileStatus[];
	selectedPath: string | null;
	onSelectFile: (path: string) => void;
	onStage: (paths: string[]) => void;
	onUnstage: (paths: string[]) => void;
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
	error,
	onClearError,
	onNavigateToDiff,
	onRefresh,
}: RemoteSourceControlProps) {
	const totalChanges = stagedFiles.length + changedFiles.length;

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
							selectedPath={selectedPath}
							onSelect={handleSelectFile}
							actionLabel="Stage"
							onAction={() => onStage([entry.path])}
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
							selectedPath={selectedPath}
							onSelect={handleSelectFile}
							actionLabel="Unstage"
							onAction={() => onUnstage([entry.path])}
						/>
					))}
				</CollapsibleSection>

				{totalChanges === 0 && (
					<div className="px-3 py-4 text-sm text-neutral-500">No changes</div>
				)}
			</div>

			{error && (
				<div className="flex items-start gap-1 px-3 py-2 text-red-400 text-xs border-t border-neutral-800">
					<span className="flex-1 break-all">{error}</span>
					<button type="button" className="shrink-0" onClick={onClearError}>
						<X className="h-3 w-3" />
					</button>
				</div>
			)}
		</div>
	);
}
