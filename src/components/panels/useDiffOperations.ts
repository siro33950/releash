import { invoke } from "@tauri-apps/api/core";
import { useCallback } from "react";
import type { DiffBase, DiffSection } from "@/types/settings";

export interface UseDiffOperationsParams {
	rootPath: string | null;
	filePath: string | null;
	section: DiffSection;
	base: DiffBase;
	onGitChanged?: () => void;
}

export interface UseDiffOperationsResult {
	handleStageGroup: (groupId: string) => Promise<void>;
	handleUnstageGroup: (groupId: string) => Promise<void>;
}

const STALE_REVIEW_GROUP_TARGET_CODE = "STALE_REVIEW_GROUP_TARGET";

function isObject(error: unknown): error is Record<string, unknown> {
	return typeof error === "object" && error !== null;
}

function isStaleReviewGroupTarget(error: unknown): boolean {
	return isObject(error) && error.code === STALE_REVIEW_GROUP_TARGET_CODE;
}

export function useDiffOperations({
	rootPath,
	filePath,
	section,
	base,
	onGitChanged,
}: UseDiffOperationsParams): UseDiffOperationsResult {
	const applyGroupAction = useCallback(
		async (command: string, groupId: string) => {
			if (!rootPath || !filePath || !groupId) return;

			try {
				await invoke(command, {
					input: {
						worktreePath: rootPath,
						path: filePath,
						section,
						base,
						groupId,
					},
				});
				onGitChanged?.();
			} catch (e) {
				if (isStaleReviewGroupTarget(e)) {
					console.warn("Review group target is stale; refreshing snapshot:", e);
					onGitChanged?.();
					return;
				}
				console.error("Group action failed:", e);
			}
		},
		[rootPath, filePath, section, base, onGitChanged],
	);

	const handleStageGroup = useCallback(
		(groupId: string) => applyGroupAction("git_stage_review_group", groupId),
		[applyGroupAction],
	);

	const handleUnstageGroup = useCallback(
		(groupId: string) => applyGroupAction("git_unstage_review_group", groupId),
		[applyGroupAction],
	);

	return {
		handleStageGroup,
		handleUnstageGroup,
	};
}
