import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

import { useBatchSpawnAgents } from "./useBatchSpawnAgents";

beforeEach(() => {
	mockInvoke.mockReset();
});

describe("useBatchSpawnAgents", () => {
	// Scenario 4: Agent設定が"none"の場合はPTYが生成されない
	it("does not invoke batch_spawn when agent is none", async () => {
		renderHook(() => useBatchSpawnAgents(["/repo"], "none", "claude", 0));
		await waitFor(() => {
			expect(mockInvoke).not.toHaveBeenCalled();
		});
	});

	it("does not invoke batch_spawn when repoPaths is empty", async () => {
		renderHook(() => useBatchSpawnAgents([], "claude", "claude", 0));
		await waitFor(() => {
			expect(mockInvoke).not.toHaveBeenCalled();
		});
	});

	it("does not invoke batch_spawn when startupCommand is empty", async () => {
		renderHook(() => useBatchSpawnAgents(["/repo"], "claude", "", 0));
		await waitFor(() => {
			expect(mockInvoke).not.toHaveBeenCalled();
		});
	});

	// 正常系: list_worktrees → batch_spawn_agent_ptys 呼び出し
	it("calls list_worktrees then batch_spawn_agent_ptys", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_worktrees")
				return Promise.resolve([{ path: "/repo/wt1", branch: "main" }]);
			if (cmd === "batch_spawn_agent_ptys")
				return Promise.resolve({ spawned: 1, failed: 0, errors: [] });
			return Promise.reject(new Error(`unexpected: ${cmd}`));
		});

		renderHook(() => useBatchSpawnAgents(["/repo"], "claude", "claude", 0));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_worktrees", {
				repoPath: "/repo",
			});
			expect(mockInvoke).toHaveBeenCalledWith("batch_spawn_agent_ptys", {
				worktreePaths: ["/repo/wt1"],
				startupCommand: "claude",
				maxConcurrent: null,
			});
		});
	});

	// Scenario 5: maxConcurrent が渡される
	it("passes maxConcurrent to batch_spawn_agent_ptys", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_worktrees")
				return Promise.resolve([{ path: "/repo/wt1", branch: "main" }]);
			if (cmd === "batch_spawn_agent_ptys")
				return Promise.resolve({ spawned: 1, failed: 0, errors: [] });
			return Promise.reject(new Error(`unexpected: ${cmd}`));
		});

		renderHook(() => useBatchSpawnAgents(["/repo"], "claude", "claude", 2));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("batch_spawn_agent_ptys", {
				worktreePaths: ["/repo/wt1"],
				startupCommand: "claude",
				maxConcurrent: 2,
			});
		});
	});

	// Scenario 7: list_worktrees 失敗時も他リポジトリは継続
	it("continues when list_worktrees fails for one repo", async () => {
		let callCount = 0;
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_worktrees") {
				callCount++;
				if (callCount === 1) return Promise.reject(new Error("fail"));
				return Promise.resolve([{ path: "/repo2/wt1", branch: "main" }]);
			}
			if (cmd === "batch_spawn_agent_ptys")
				return Promise.resolve({ spawned: 1, failed: 0, errors: [] });
			return Promise.reject(new Error(`unexpected: ${cmd}`));
		});

		const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

		renderHook(() =>
			useBatchSpawnAgents(["/repo1", "/repo2"], "claude", "claude", 0),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("batch_spawn_agent_ptys", {
				worktreePaths: ["/repo2/wt1"],
				startupCommand: "claude",
				maxConcurrent: null,
			});
			expect(warnSpy).toHaveBeenCalled();
		});

		warnSpy.mockRestore();
	});

	// Scenario 7: batch_spawn_agent_ptys の部分失敗をログ出力
	it("logs warning when batch_spawn reports failures", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_worktrees")
				return Promise.resolve([
					{ path: "/repo/wt1", branch: "main" },
					{ path: "/repo/wt2", branch: "dev" },
				]);
			if (cmd === "batch_spawn_agent_ptys")
				return Promise.resolve({
					spawned: 1,
					failed: 1,
					errors: ["PTY spawn failed for /repo/wt2"],
				});
			return Promise.reject(new Error(`unexpected: ${cmd}`));
		});

		const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

		renderHook(() => useBatchSpawnAgents(["/repo"], "claude", "claude", 0));

		await waitFor(() => {
			expect(warnSpy).toHaveBeenCalledWith(
				"[batch_spawn] 1 spawned, 1 failed",
				["PTY spawn failed for /repo/wt2"],
			);
		});

		warnSpy.mockRestore();
	});

	// 複数リポジトリの worktree が並行取得されフラット化される
	it("fetches worktrees from multiple repos in parallel", async () => {
		mockInvoke.mockImplementation((cmd: string, args: unknown) => {
			if (cmd === "list_worktrees") {
				const { repoPath } = args as { repoPath: string };
				if (repoPath === "/repo1")
					return Promise.resolve([{ path: "/repo1/wt1", branch: "main" }]);
				if (repoPath === "/repo2")
					return Promise.resolve([
						{ path: "/repo2/wt1", branch: "main" },
						{ path: "/repo2/wt2", branch: "dev" },
					]);
			}
			if (cmd === "batch_spawn_agent_ptys")
				return Promise.resolve({ spawned: 3, failed: 0, errors: [] });
			return Promise.reject(new Error(`unexpected: ${cmd}`));
		});

		renderHook(() =>
			useBatchSpawnAgents(["/repo1", "/repo2"], "claude", "claude", 0),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("batch_spawn_agent_ptys", {
				worktreePaths: ["/repo1/wt1", "/repo2/wt1", "/repo2/wt2"],
				startupCommand: "claude",
				maxConcurrent: null,
			});
		});
	});
});
