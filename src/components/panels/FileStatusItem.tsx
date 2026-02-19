import { Minus, Pencil, Plus } from "lucide-react";
import { cn } from "@/lib/utils";
import type { GitFileStatus } from "@/types/git";

export function statusColor(status: string): string {
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

export function StatusIcon({ status }: { status: string }) {
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

export function formatPath(path: string): { dir: string; name: string } {
	const parts = path.split("/");
	const name = parts.pop() ?? path;
	const dir = parts.length > 0 ? `${parts.join("/")}/` : "";
	return { dir, name };
}

export interface FileStatusItemProps {
	entry: GitFileStatus;
	statusField: "index_status" | "worktree_status";
	selected?: boolean;
	onSelect?: (entry: GitFileStatus) => void;
	actionLabel: string;
	onAction: () => void;
	alwaysShowAction?: boolean;
}

export function FileStatusItem({
	entry,
	statusField,
	selected,
	onSelect,
	actionLabel,
	onAction,
	alwaysShowAction,
}: FileStatusItemProps) {
	const status = entry[statusField];
	const { dir, name } = formatPath(entry.path);

	return (
		// biome-ignore lint/a11y/useSemanticElements: outer element cannot be <button> because it contains a nested <button> for the action
		<div
			role="button"
			tabIndex={0}
			className={cn(
				"group flex w-full items-center gap-1.5 px-4 py-1 text-sm transition-colors",
				selected ? "bg-sidebar-accent" : "hover:bg-sidebar-accent",
			)}
			onClick={() => onSelect?.(entry)}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					onSelect?.(entry);
				}
			}}
		>
			<StatusIcon status={status} />
			<span className="truncate flex-1 text-left">
				<span className="text-muted-foreground">{dir}</span>
				<span className="font-semibold">{name}</span>
			</span>
			<button
				type="button"
				className={cn(
					"items-center justify-center h-5 w-5 rounded hover:bg-sidebar-accent-foreground/10 shrink-0",
					alwaysShowAction ? "inline-flex" : "hidden group-hover:inline-flex",
				)}
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
