import { invoke } from "@tauri-apps/api/core";
import { useCallback } from "react";
import type { DiffBase, DiffSection } from "@/types/settings";

export interface UseDiffOperationsParams {
	rootPath: string | null;
	filePath: string | null;
	section: DiffSection;
	base: DiffBase;
	snapshotVersion: number | null;
	onGitChanged?: () => void;
}

export interface UseDiffOperationsResult {
	handleStageGroup: (groupIndex: number) => Promise<void>;
	handleUnstageGroup: (groupIndex: number) => Promise<void>;
}

export function useDiffOperations({
	rootPath,
	filePath,
	section,
	base,
	snapshotVersion,
	onGitChanged,
}: UseDiffOperationsParams): UseDiffOperationsResult {
	const applyGroupAction = useCallback(
		async (command: string, groupIndex: number) => {
			if (!rootPath || !filePath || snapshotVersion == null) return;

			try {
				await invoke(command, {
					input: {
						worktreePath: rootPath,
						path: filePath,
						section,
						base,
						groupIndex,
						snapshotVersion,
					},
				});
				onGitChanged?.();
			} catch (e) {
				console.error("Group action failed:", e);
			}
		},
		[rootPath, filePath, section, base, snapshotVersion, onGitChanged],
	);

	const handleStageGroup = useCallback(
		(groupIndex: number) =>
			applyGroupAction("git_stage_review_group", groupIndex),
		[applyGroupAction],
	);

	const handleUnstageGroup = useCallback(
		(groupIndex: number) =>
			applyGroupAction("git_unstage_review_group", groupIndex),
		[applyGroupAction],
	);

	return {
		handleStageGroup,
		handleUnstageGroup,
	};
}
