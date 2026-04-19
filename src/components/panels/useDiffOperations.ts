import { invoke } from "@tauri-apps/api/core";
import { useCallback } from "react";
import type { ChangeGroup, Hunk } from "@/lib/computeHunks";

interface DiffHunksResult {
	hunks: Hunk[];
	changeGroups: ChangeGroup[];
}

export interface UseDiffOperationsParams {
	filePath: string;
	rootPath: string | null;
	originalContent: string;
	modifiedContent: string;
	onStageHunk?: (rootPath: string, patch: string) => Promise<void>;
	onUnstageHunk?: (rootPath: string, patch: string) => Promise<void>;
	onGitChanged?: () => void;
}

export interface UseDiffOperationsResult {
	handleStageGroup: (groupIndex: number) => Promise<void>;
	handleUnstageGroup: (groupIndex: number) => Promise<void>;
}

export function useDiffOperations({
	filePath,
	rootPath,
	originalContent,
	modifiedContent,
	onStageHunk,
	onUnstageHunk,
	onGitChanged,
}: UseDiffOperationsParams): UseDiffOperationsResult {
	const applyGroupAction = useCallback(
		async (
			groupIndex: number,
			action: ((rootPath: string, patch: string) => Promise<void>) | undefined,
		) => {
			if (!rootPath || !action) return;

			try {
				const relativePath = await invoke<string | null>("get_relative_path", {
					rootPath,
					filePath,
				});
				if (!relativePath) return;

				const result = await invoke<DiffHunksResult>("compute_diff_hunks", {
					original: originalContent,
					modified: modifiedContent,
					filePath: relativePath,
				});
				const group = result.changeGroups.find(
					(g) => g.groupIndex === groupIndex,
				);
				if (!group) return;
				const hunk = result.hunks.find((h) => h.index === group.hunkIndex);
				if (!hunk) return;

				const patch = await invoke<string>("generate_group_patch", {
					filePath: relativePath,
					hunk,
					group,
				});
				if (patch) {
					await action(rootPath, patch);
					onGitChanged?.();
				}
			} catch (e) {
				console.error("Group action failed:", e);
			}
		},
		[rootPath, filePath, originalContent, modifiedContent, onGitChanged],
	);

	const handleStageGroup = useCallback(
		(groupIndex: number) => applyGroupAction(groupIndex, onStageHunk),
		[applyGroupAction, onStageHunk],
	);

	const handleUnstageGroup = useCallback(
		(groupIndex: number) => applyGroupAction(groupIndex, onUnstageHunk),
		[applyGroupAction, onUnstageHunk],
	);

	return {
		handleStageGroup,
		handleUnstageGroup,
	};
}
