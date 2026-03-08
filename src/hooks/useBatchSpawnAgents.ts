import { invoke } from "@tauri-apps/api/core";
import { useEffect } from "react";
import type { WorktreeEntry } from "@/types/git";
import type { AgentType } from "@/types/settings";

export function useBatchSpawnAgents(
	repoPaths: string[],
	agentType: AgentType,
	startupCommand: string,
	maxConcurrent: number,
) {
	useEffect(() => {
		if (repoPaths.length === 0) return;
		if (agentType === "none") return;
		if (!startupCommand) return;

		const normalizedMaxConcurrent =
			Number.isInteger(maxConcurrent) && maxConcurrent > 0
				? maxConcurrent
				: null;

		let cancelled = false;

		const batchSpawn = async () => {
			const worktreeResults = await Promise.all(
				repoPaths.map(async (repoPath) => {
					try {
						const wts = await invoke<WorktreeEntry[]>("list_worktrees", {
							repoPath,
						});
						return wts.map((wt) => wt.path);
					} catch (e) {
						console.warn(
							`[batch_spawn] list_worktrees failed for ${repoPath}:`,
							e,
						);
						return [];
					}
				}),
			);
			const allWorktreePaths = worktreeResults.flat();

			if (cancelled || allWorktreePaths.length === 0) return;

			try {
				const result = await invoke<{
					spawned: number;
					failed: number;
					errors: string[];
				}>("batch_spawn_agent_ptys", {
					worktreePaths: allWorktreePaths,
					startupCommand,
					maxConcurrent: normalizedMaxConcurrent,
				});
				if (result.failed > 0) {
					console.warn(
						`[batch_spawn] ${result.spawned} spawned, ${result.failed} failed`,
						result.errors,
					);
				}
			} catch (e) {
				console.warn("[batch_spawn] batch_spawn_agent_ptys failed:", e);
			}
		};

		batchSpawn();

		return () => {
			cancelled = true;
		};
	}, [repoPaths, agentType, startupCommand, maxConcurrent]);
}
