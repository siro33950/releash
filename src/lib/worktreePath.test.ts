import { describe, expect, it } from "vitest";
import { branchToDir, computeWorktreeDir } from "./worktreePath";

describe("computeWorktreeDir", () => {
	it("computes worktree directory from repo path", () => {
		expect(computeWorktreeDir("/home/user/projects/my-repo")).toBe(
			"/home/user/projects/my-repo-worktrees",
		);
	});

	it("handles trailing slash in repo path", () => {
		expect(computeWorktreeDir("/home/user/projects/my-repo/")).toBe(
			"/home/user/projects/my-repo-worktrees",
		);
	});
});

describe("branchToDir", () => {
	it("replaces slashes with hyphens", () => {
		expect(branchToDir("feat/issues/305")).toBe("feat-issues-305");
	});

	it("returns branch name as-is when no slashes", () => {
		expect(branchToDir("main")).toBe("main");
	});
});
