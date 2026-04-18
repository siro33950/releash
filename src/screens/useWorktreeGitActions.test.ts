import { describe, expect, it } from "vitest";
import {
	type GitState,
	gitReducer,
	initialUIState,
	uiReducer,
} from "./useWorktreeGitActions";

describe("uiReducer", () => {
	it("SET_SETTINGS_OPEN updates isSettingsOpen", () => {
		const state = uiReducer(initialUIState, {
			type: "SET_SETTINGS_OPEN",
			open: true,
		});
		expect(state.isSettingsOpen).toBe(true);
	});

	it("OPEN_CREATE_BRANCH sets showCreateBranch=true and resets newBranchName", () => {
		const prev = {
			...initialUIState,
			newBranchName: "old-name",
		};
		const state = uiReducer(prev, { type: "OPEN_CREATE_BRANCH" });
		expect(state.showCreateBranch).toBe(true);
		expect(state.newBranchName).toBe("");
	});

	it("CLOSE_CREATE_BRANCH sets showCreateBranch=false", () => {
		const prev = { ...initialUIState, showCreateBranch: true };
		const state = uiReducer(prev, { type: "CLOSE_CREATE_BRANCH" });
		expect(state.showCreateBranch).toBe(false);
	});

	it("SET_NEW_BRANCH_NAME updates newBranchName", () => {
		const state = uiReducer(initialUIState, {
			type: "SET_NEW_BRANCH_NAME",
			name: "feat/new",
		});
		expect(state.newBranchName).toBe("feat/new");
	});
});

describe("gitReducer", () => {
	const initialGitState: GitState = {
		gitError: null,
		refreshKey: 0,
	};

	it("SET_GIT_ERROR updates gitError", () => {
		const state = gitReducer(initialGitState, {
			type: "SET_GIT_ERROR",
			error: "push failed",
		});
		expect(state.gitError).toBe("push failed");
	});

	it("SET_GIT_ERROR clears gitError with null", () => {
		const prev: GitState = { ...initialGitState, gitError: "some error" };
		const state = gitReducer(prev, { type: "SET_GIT_ERROR", error: null });
		expect(state.gitError).toBeNull();
	});

	it("REFRESH increments refreshKey by 1", () => {
		const state = gitReducer(initialGitState, { type: "REFRESH" });
		expect(state.refreshKey).toBe(1);

		const state2 = gitReducer(state, { type: "REFRESH" });
		expect(state2.refreshKey).toBe(2);
	});
});
