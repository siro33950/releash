import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
	type GitState,
	gitReducer,
	initialUIState,
	uiReducer,
	useWorktreeGitActions,
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

it.each(["stage", "unstage", "createBranch"] as const)(
	"メニューの%s失敗はダイアログ用errorだけに渡す",
	async (operation) => {
		const failure = vi.fn().mockRejectedValue(new Error(`${operation} failed`));
		const dispatchGit = vi.fn();
		const notice = vi.fn();
		window.addEventListener("releash-client-error", notice);
		try {
			const { result } = renderHook(() =>
				useWorktreeGitActions({
					rootPath: "/repo",
					stage: operation === "stage" ? failure : vi.fn(),
					unstage: operation === "unstage" ? failure : vi.fn(),
					createBranch: operation === "createBranch" ? failure : vi.fn(),
					newBranchName: "feature",
					dispatchGit,
					dispatchUI: vi.fn(),
				}),
			);
			await (operation === "stage"
				? result.current.handleGitStageAll()
				: operation === "unstage"
					? result.current.handleGitUnstageAll()
					: result.current.executeCreateBranch());
			expect(dispatchGit).toHaveBeenCalledExactlyOnceWith({
				type: "SET_GIT_ERROR",
				error: `${operation} failed`,
			});
			expect(notice).not.toHaveBeenCalled();
		} finally {
			window.removeEventListener("releash-client-error", notice);
		}
	},
);

describe("gitReducer", () => {
	const initialGitState: GitState = {
		gitError: null,
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
});
