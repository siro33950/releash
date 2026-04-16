import { describe, expect, it } from "vitest";
import { buildWorkspaceState, worktreeNameFromPath } from "./workspace-state";

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

		const result = buildWorkspaceState(internal, "agent", true, true);
		expect(result.layout.rightBottomCollapsed).toBe(true);
		expect(result.layout.centerTab).toBe("agent");
	});

	it("rightBottomActiveTabが保持される", () => {
		const internal = {
			tabs: [],
			activeEditorPath: null,
			activeView: "git",
			rightBottomCollapsed: false,
			rightBottomActiveTab: "comments",
		};

		const result = buildWorkspaceState(internal, "editor", true, true);
		expect(result.layout.rightBottomActiveTab).toBe("comments");
	});
});
