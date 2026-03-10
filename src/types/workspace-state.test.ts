import { describe, expect, it } from "vitest";
import {
	buildWorkspaceState,
	migrateWorkspaceState,
	type WorkspaceState,
	worktreeNameFromPath,
} from "./workspace-state";

describe("worktreeNameFromPath", () => {
	it("通常パスから末尾のディレクトリ名を返す", () => {
		expect(worktreeNameFromPath("/path/to/repo")).toBe("repo");
	});

	it("ルートパスでは空文字を返す（pop結果）", () => {
		expect(worktreeNameFromPath("/")).toBe("");
	});

	it("スラッシュなしのパスをそのまま返す", () => {
		expect(worktreeNameFromPath("repo")).toBe("repo");
	});

	it("深いネストのパスから末尾を返す", () => {
		expect(worktreeNameFromPath("/home/user/projects/my-app")).toBe("my-app");
	});
});

describe("buildWorkspaceState", () => {
	it("全フィールドが正しくマッピングされる", () => {
		const internal = {
			tabs: [
				{ path: "/repo/src/main.rs", name: "main.rs" },
				{ path: "/repo/src/lib.rs", name: "lib.rs" },
			],
			activeEditorPath: "/repo/src/main.rs",
			activeView: "git",
			rightBottomCollapsed: false,
			rightBottomActiveTab: "terminal",
		};

		const result = buildWorkspaceState(internal, "editor", true, true);

		expect(result).toEqual({
			version: 1,
			tabs: {
				editors: internal.tabs,
				activeEditorPath: "/repo/src/main.rs",
			},
			layout: {
				centerTab: "editor",
				activeView: "git",
				leftNavCollapsed: false,
				rightCollapsed: false,
				rightBottomCollapsed: false,
				rightBottomActiveTab: "terminal",
				workflowPanelRatios: undefined,
			},
		});
	});

	it("leftNavVisible=true → leftNavCollapsed=false（反転）", () => {
		const internal = {
			tabs: [],
			activeEditorPath: null,
			activeView: "git",
			rightBottomCollapsed: false,
			rightBottomActiveTab: "terminal",
		};

		const result = buildWorkspaceState(internal, "editor", true, true);
		expect(result.layout.leftNavCollapsed).toBe(false);
	});

	it("leftNavVisible=false → leftNavCollapsed=true（反転）", () => {
		const internal = {
			tabs: [],
			activeEditorPath: null,
			activeView: "git",
			rightBottomCollapsed: false,
			rightBottomActiveTab: "terminal",
		};

		const result = buildWorkspaceState(internal, "editor", false, true);
		expect(result.layout.leftNavCollapsed).toBe(true);
	});

	it("rightVisible=false → rightCollapsed=true（反転）", () => {
		const internal = {
			tabs: [],
			activeEditorPath: null,
			activeView: "git",
			rightBottomCollapsed: false,
			rightBottomActiveTab: "terminal",
		};

		const result = buildWorkspaceState(internal, "editor", true, false);
		expect(result.layout.rightCollapsed).toBe(true);
	});

	it("rightBottomCollapsedが保持される", () => {
		const internal = {
			tabs: [],
			activeEditorPath: null,
			activeView: "git",
			rightBottomCollapsed: true,
			rightBottomActiveTab: "terminal",
		};

		const result = buildWorkspaceState(internal, "workflow", true, true);
		expect(result.layout.rightBottomCollapsed).toBe(true);
		expect(result.layout.centerTab).toBe("workflow");
	});

	it("rightBottomActiveTabが保持される", () => {
		const internal = {
			tabs: [],
			activeEditorPath: null,
			activeView: "git",
			rightBottomCollapsed: false,
			rightBottomActiveTab: "comment" as const,
		};

		const result = buildWorkspaceState(internal, "editor", true, true);
		expect(result.layout.rightBottomActiveTab).toBe("comment");
	});

	it("workflowPanelRatios を含める", () => {
		const internal = {
			tabs: [],
			activeEditorPath: null,
			activeView: "git",
			rightBottomCollapsed: false,
			rightBottomActiveTab: "terminal",
			workflowPanelRatios: [60, 40] as [number, number],
		};

		const result = buildWorkspaceState(internal, "workflow", true, true);
		expect(result.layout.workflowPanelRatios).toEqual([60, 40]);
	});

	it("workflowPanelRatios が未設定の場合 undefined", () => {
		const internal = {
			tabs: [],
			activeEditorPath: null,
			activeView: "git",
			rightBottomCollapsed: false,
			rightBottomActiveTab: "terminal",
		};

		const result = buildWorkspaceState(internal, "editor", true, true);
		expect(result.layout.workflowPanelRatios).toBeUndefined();
	});
});

describe("migrateWorkspaceState", () => {
	function makeState(centerTab: string): WorkspaceState {
		return {
			version: 1,
			tabs: { editors: [], activeEditorPath: null },
			layout: {
				centerTab: centerTab as "editor" | "workflow",
				activeView: "git",
				leftNavCollapsed: false,
				rightCollapsed: false,
				rightBottomCollapsed: false,
			},
		};
	}

	it('"agent" → "workflow" に変換する', () => {
		const state = makeState("agent");
		const migrated = migrateWorkspaceState(state);
		expect(migrated.layout.centerTab).toBe("workflow");
	});

	it('"editor" はそのまま維持する', () => {
		const state = makeState("editor");
		const migrated = migrateWorkspaceState(state);
		expect(migrated.layout.centerTab).toBe("editor");
	});

	it('"workflow" はそのまま維持する', () => {
		const state = makeState("workflow");
		const migrated = migrateWorkspaceState(state);
		expect(migrated.layout.centerTab).toBe("workflow");
	});

	it("元のオブジェクトを変更しない", () => {
		const state = makeState("agent");
		const migrated = migrateWorkspaceState(state);
		expect(migrated).not.toBe(state);
		expect(state.layout.centerTab as string).toBe("agent");
	});

	it('"review" → "comment" に変換する', () => {
		const state: WorkspaceState = {
			...makeState("editor"),
			layout: {
				...makeState("editor").layout,
				rightBottomActiveTab:
					"review" as unknown as WorkspaceState["layout"]["rightBottomActiveTab"],
			},
		};
		const migrated = migrateWorkspaceState(state);
		expect(migrated.layout.rightBottomActiveTab).toBe("comment");
	});

	it("既に comment のタブはそのまま維持する", () => {
		const state: WorkspaceState = {
			...makeState("editor"),
			layout: {
				...makeState("editor").layout,
				rightBottomActiveTab: "comment",
			},
		};
		const migrated = migrateWorkspaceState(state);
		expect(migrated.layout.rightBottomActiveTab).toBe("comment");
	});
});
