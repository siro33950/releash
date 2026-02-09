import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import type { BranchInfo, WorktreeEntry } from "@/types/git";

interface CreateWorktreeDialogProps {
	open: boolean;
	repoPath: string;
	worktreeRoot: string;
	existingWorktrees: WorktreeEntry[];
	onCreated: (entry: WorktreeEntry) => void;
	onCancel: () => void;
}

function branchToDir(branch: string): string {
	return branch.replace(/\//g, "-");
}

export function CreateWorktreeDialog({
	open,
	repoPath,
	worktreeRoot,
	existingWorktrees,
	onCreated,
	onCancel,
}: CreateWorktreeDialogProps) {
	const [branches, setBranches] = useState<BranchInfo[]>([]);
	const [filter, setFilter] = useState("");
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [branchName, setBranchName] = useState("");
	const [creating, setCreating] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const listRef = useRef<HTMLDivElement>(null);

	const existingBranches = useMemo(
		() => new Set(existingWorktrees.map((w) => w.branch)),
		[existingWorktrees],
	);

	useEffect(() => {
		if (!open) return;
		setFilter("");
		setBranchName("");
		setSelectedIndex(0);
		setError(null);
		invoke<BranchInfo[]>("list_branches", { filePath: repoPath })
			.then(setBranches)
			.catch(() => setBranches([]));
	}, [open, repoPath]);

	const filteredBranches = useMemo(() => {
		const lower = filter.toLowerCase();
		const filtered = filter
			? branches.filter((b) => b.name.toLowerCase().includes(lower))
			: branches;
		const local = filtered.filter((b) => !b.is_remote);
		const remote = filtered.filter((b) => b.is_remote);
		return { local, remote };
	}, [branches, filter]);

	const allBranchNames = useMemo(() => branches.map((b) => b.name), [branches]);

	const canCreateNew = filter.length > 0 && !allBranchNames.includes(filter);

	const flatList = useMemo(() => {
		const items: { branch: BranchInfo; isNew: boolean; section?: string }[] =
			[];
		if (canCreateNew) {
			items.push({
				branch: { name: filter, is_remote: false },
				isNew: true,
			});
		}
		if (filteredBranches.local.length > 0) {
			for (const b of filteredBranches.local) {
				items.push({ branch: b, isNew: false, section: "local" });
			}
		}
		if (filteredBranches.remote.length > 0) {
			for (const b of filteredBranches.remote) {
				items.push({ branch: b, isNew: false, section: "remote" });
			}
		}
		return items;
	}, [filteredBranches, canCreateNew, filter]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: reset index when filter changes
	useEffect(() => {
		setSelectedIndex(0);
	}, [filter]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: scroll when selection changes
	useEffect(() => {
		if (!listRef.current) return;
		const active = listRef.current.querySelector("[data-active='true']");
		active?.scrollIntoView({ block: "nearest" });
	}, [selectedIndex]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "ArrowDown") {
				e.preventDefault();
				setSelectedIndex((i) => Math.min(i + 1, flatList.length - 1));
			} else if (e.key === "ArrowUp") {
				e.preventDefault();
				setSelectedIndex((i) => Math.max(i - 1, 0));
			} else if (e.key === "Enter") {
				e.preventDefault();
				const item = flatList[selectedIndex];
				if (!item) return;
				if (item.isNew) {
					setBranchName(filter);
				} else if (!existingBranches.has(item.branch.name)) {
					setBranchName(item.branch.name);
				}
			}
		},
		[flatList, selectedIndex, filter, existingBranches],
	);

	const selectBranch = useCallback(
		(name: string, isNew: boolean) => {
			if (!isNew && existingBranches.has(name)) return;
			setBranchName(name);
		},
		[existingBranches],
	);

	const worktreePath = branchName
		? `${worktreeRoot}/${branchToDir(branchName)}`
		: "";
	const isNewBranch =
		branchName.length > 0 && !allBranchNames.includes(branchName);

	const handleCreate = useCallback(async () => {
		if (!branchName || !worktreePath) return;
		setCreating(true);
		setError(null);
		try {
			const entry = await invoke<WorktreeEntry>("create_worktree", {
				repoPath,
				worktreePath,
				branch: branchName,
				createBranch: isNewBranch,
				baseBranch: isNewBranch ? "HEAD" : null,
			});
			onCreated(entry);
		} catch (e) {
			setError(String(e));
		} finally {
			setCreating(false);
		}
	}, [branchName, worktreePath, repoPath, isNewBranch, onCreated]);

	if (branchName) {
		return (
			<AlertDialog open={open} onOpenChange={(o) => !o && onCancel()}>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>New Workspace</AlertDialogTitle>
						<AlertDialogDescription>
							{isNewBranch
								? `Create new branch "${branchName}" and workspace`
								: `Create workspace for branch "${branchName}"`}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<div className="grid gap-3 text-sm">
						<div className="flex items-center gap-2">
							<span className="text-muted-foreground w-16 shrink-0">
								Branch:
							</span>
							<span className="font-mono truncate">{branchName}</span>
						</div>
						<div className="flex items-center gap-2">
							<span className="text-muted-foreground w-16 shrink-0">Path:</span>
							<span className="font-mono text-xs truncate">{worktreePath}</span>
						</div>
						{error && <p className="text-sm text-destructive">{error}</p>}
					</div>
					<AlertDialogFooter>
						<AlertDialogCancel
							onClick={() => {
								setBranchName("");
								setError(null);
							}}
							disabled={creating}
						>
							Back
						</AlertDialogCancel>
						<AlertDialogAction onClick={handleCreate} disabled={creating}>
							{creating ? "Creating..." : "Create"}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		);
	}

	let lastSection: string | undefined;

	return (
		<AlertDialog open={open} onOpenChange={(o) => !o && onCancel()}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>New Workspace</AlertDialogTitle>
					<AlertDialogDescription>
						Select an existing branch or type a new branch name
					</AlertDialogDescription>
				</AlertDialogHeader>
				<div className="grid gap-2">
					<Input
						placeholder="Filter or create branch..."
						value={filter}
						onChange={(e) => setFilter(e.target.value)}
						onKeyDown={handleKeyDown}
						autoFocus
					/>
					<div
						ref={listRef}
						className="max-h-48 overflow-y-auto rounded border border-border"
					>
						{flatList.map((item, idx) => {
							const showHeader =
								!item.isNew &&
								item.section !== lastSection &&
								item.section != null;
							if (!item.isNew && item.section != null) {
								lastSection = item.section;
							}
							const isExisting = existingBranches.has(item.branch.name);

							return (
								<div key={item.isNew ? `__new__` : item.branch.name}>
									{showHeader && (
										<div className="px-3 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground bg-muted/50 sticky top-0">
											{item.section === "local" ? "Local" : "Remote"}
										</div>
									)}
									{item.isNew ? (
										<button
											type="button"
											data-active={selectedIndex === idx}
											className="flex w-full items-center gap-2 px-3 py-1.5 text-sm hover:bg-accent data-[active=true]:bg-accent"
											onClick={() => selectBranch(filter, true)}
										>
											<span className="text-primary">+</span>
											<span>Create branch &quot;{filter}&quot;</span>
										</button>
									) : (
										<button
											type="button"
											data-active={selectedIndex === idx}
											disabled={isExisting}
											className="flex w-full items-center gap-2 px-3 py-1.5 text-sm hover:bg-accent data-[active=true]:bg-accent disabled:opacity-40 disabled:cursor-not-allowed"
											onClick={() => selectBranch(item.branch.name, false)}
										>
											<span className="truncate">{item.branch.name}</span>
											{isExisting && (
												<span className="ml-auto text-xs text-muted-foreground shrink-0">
													already open
												</span>
											)}
										</button>
									)}
								</div>
							);
						})}
						{flatList.length === 0 && (
							<div className="px-3 py-4 text-sm text-muted-foreground text-center">
								No branches found
							</div>
						)}
					</div>
				</div>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={onCancel}>Cancel</AlertDialogCancel>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
