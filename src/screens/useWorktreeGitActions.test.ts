import { describe, expect, it } from "vitest";
import {
	type EditorState,
	editorReducer,
	type GitState,
	gitReducer,
	initialEditorState,
	initialUIState,
	type UIState,
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

	it("SET_CLOSING_TAB updates closingTabPath", () => {
		const state = uiReducer(initialUIState, {
			type: "SET_CLOSING_TAB",
			path: "src/main.ts",
		});
		expect(state.closingTabPath).toBe("src/main.ts");
	});

	it("SET_CLOSING_TAB resets to null", () => {
		const prev: UIState = { ...initialUIState, closingTabPath: "src/main.ts" };
		const state = uiReducer(prev, { type: "SET_CLOSING_TAB", path: null });
		expect(state.closingTabPath).toBeNull();
	});

	it("SET_SAVING_CONFLICT updates savingConflictPath", () => {
		const state = uiReducer(initialUIState, {
			type: "SET_SAVING_CONFLICT",
			path: "conflict.ts",
		});
		expect(state.savingConflictPath).toBe("conflict.ts");
	});

	it("SET_DISCARD_CONFIRM updates showDiscardConfirm", () => {
		const state = uiReducer(initialUIState, {
			type: "SET_DISCARD_CONFIRM",
			show: true,
		});
		expect(state.showDiscardConfirm).toBe(true);
	});

	it("OPEN_CREATE_BRANCH sets showCreateBranch=true and resets newBranchName", () => {
		const prev: UIState = {
			...initialUIState,
			newBranchName: "old-name",
		};
		const state = uiReducer(prev, { type: "OPEN_CREATE_BRANCH" });
		expect(state.showCreateBranch).toBe(true);
		expect(state.newBranchName).toBe("");
	});

	it("CLOSE_CREATE_BRANCH sets showCreateBranch=false", () => {
		const prev: UIState = { ...initialUIState, showCreateBranch: true };
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

	it("SET_EDITOR_DRAG_OVER updates editorDragOver", () => {
		const state = uiReducer(initialUIState, {
			type: "SET_EDITOR_DRAG_OVER",
			value: true,
		});
		expect(state.editorDragOver).toBe(true);
	});
});

describe("gitReducer", () => {
	const initialGitState: GitState = {
		diffBase: "staged",
		diffMode: "inline",
		gitError: null,
		refreshKey: 0,
	};

	it("SET_DIFF_BASE updates diffBase", () => {
		const state = gitReducer(initialGitState, {
			type: "SET_DIFF_BASE",
			value: "HEAD",
		});
		expect(state.diffBase).toBe("HEAD");
	});

	it("SET_DIFF_MODE updates diffMode", () => {
		const state = gitReducer(initialGitState, {
			type: "SET_DIFF_MODE",
			value: "split",
		});
		expect(state.diffMode).toBe("split");
	});

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

describe("editorReducer", () => {
	it("SET_ACTIVE_VIEW updates activeView", () => {
		const state = editorReducer(initialEditorState, {
			type: "SET_ACTIVE_VIEW",
			view: "files",
		});
		expect(state.activeView).toBe("files");
	});

	it("TRIGGER_SEARCH sets activeView, searchInitialQuery, and increments searchFocusKey", () => {
		const state = editorReducer(initialEditorState, {
			type: "TRIGGER_SEARCH",
			query: "hello",
		});
		expect(state.activeView).toBe("search");
		expect(state.searchInitialQuery).toBe("hello");
		expect(state.searchFocusKey).toBe(1);
	});

	it("TRIGGER_SEARCH increments searchFocusKey on successive calls", () => {
		const state1 = editorReducer(initialEditorState, {
			type: "TRIGGER_SEARCH",
			query: "a",
		});
		const state2 = editorReducer(state1, {
			type: "TRIGGER_SEARCH",
			query: "b",
		});
		expect(state2.searchFocusKey).toBe(2);
		expect(state2.searchInitialQuery).toBe("b");
	});

	it("SET_PENDING_REVEAL updates pendingReveal", () => {
		const reveal = { path: "src/main.ts", line: 42 };
		const state = editorReducer(initialEditorState, {
			type: "SET_PENDING_REVEAL",
			reveal,
		});
		expect(state.pendingReveal).toEqual(reveal);
	});

	it("SET_PENDING_REVEAL clears with null", () => {
		const prev: EditorState = {
			...initialEditorState,
			pendingReveal: { path: "a.ts", line: 1 },
		};
		const state = editorReducer(prev, {
			type: "SET_PENDING_REVEAL",
			reveal: null,
		});
		expect(state.pendingReveal).toBeNull();
	});

	it("INCREMENT_NEW_FOLDER increments newFolderKey by 1", () => {
		const state = editorReducer(initialEditorState, {
			type: "INCREMENT_NEW_FOLDER",
		});
		expect(state.newFolderKey).toBe(1);

		const state2 = editorReducer(state, { type: "INCREMENT_NEW_FOLDER" });
		expect(state2.newFolderKey).toBe(2);
	});
});
