import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceState } from "@/types/workspace-state";

const mockGetState = vi.fn<(rootPath: string) => WorkspaceState | undefined>();
const mockLoadState =
	vi.fn<(rootPath: string) => Promise<WorkspaceState | undefined>>();
const mockUpdateState =
	vi.fn<(rootPath: string, state: WorkspaceState) => void>();
const mockFlushState = vi.fn<(rootPath: string) => void>();

vi.mock("@/hooks/useWorkspaceStateCache", () => ({
	useWorkspaceStateCache: () => ({
		getState: mockGetState,
		loadState: mockLoadState,
		updateState: mockUpdateState,
		flushState: mockFlushState,
	}),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

import type { InternalWorktreeState } from "@/types/workspace-state";
import { useWorkspacePersistence } from "./useWorkspacePersistence";

function makeState(overrides?: Partial<WorkspaceState>): WorkspaceState {
	return {
		version: 1,
		tabs: {
			editors: [
				{ path: "/repoA/src/main.rs", name: "main.rs" },
				{ path: "/repoA/src/lib.rs", name: "lib.rs" },
			],
			activeEditorPath: "/repoA/src/main.rs",
		},
		layout: {
			centerTab: "editor",
			activeView: "git",
			leftNavCollapsed: false,
			rightCollapsed: false,
			rightBottomCollapsed: false,
		},
		...overrides,
	};
}

function makeInternalState(
	overrides?: Partial<InternalWorktreeState>,
): InternalWorktreeState {
	return {
		tabs: [
			{ path: "/repoA/src/main.rs", name: "main.rs" },
			{ path: "/repoA/src/lib.rs", name: "lib.rs" },
		],
		activeEditorPath: "/repoA/src/main.rs",
		activeView: "git",
		rightBottomCollapsed: false,
		rightBottomActiveTab: "terminal",
		...overrides,
	};
}

function makePanelRef() {
	return {
		current: {
			collapse: vi.fn(),
			expand: vi.fn(),
			isCollapsed: vi.fn(() => false),
			isExpanded: vi.fn(() => true),
			resize: vi.fn(),
			getSize: vi.fn(() => ({ asPercentage: 50, inPixels: 500 })),
			getId: vi.fn(() => "panel"),
		},
	};
}

describe("useWorkspacePersistence", () => {
	let rafCallbacks: Array<() => void>;

	beforeEach(() => {
		vi.clearAllMocks();
		mockLoadState.mockResolvedValue(undefined);
		rafCallbacks = [];
		vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
			rafCallbacks.push(cb);
			return rafCallbacks.length;
		});
	});

	afterEach(() => {
		vi.unstubAllGlobals();
	});

	function flushRAF() {
		for (const cb of rafCallbacks) cb();
		rafCallbacks = [];
	}

	it("シナリオ1: Worktree切替時に現在の状態が自動保存される", () => {
		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const { result, rerender } = renderHook(
			({ selectedRootPath }) =>
				useWorkspacePersistence({
					selectedRootPath,
					centerTab: "editor",
					leftNavVisible: true,
					rightVisible: true,
					setCenterTab,
					leftNavRef,
					rightPanelRef,
				}),
			{ initialProps: { selectedRootPath: "/repoA" as string | null } },
		);

		// Set internal state via Map
		act(() => {
			result.current.internalStateMapRef.current.set(
				"/repoA",
				makeInternalState(),
			);
		});

		// Switch worktree
		rerender({ selectedRootPath: "/repoB" });

		expect(mockUpdateState).toHaveBeenCalledWith(
			"/repoA",
			expect.objectContaining({
				version: 1,
				tabs: expect.objectContaining({
					editors: [
						{ path: "/repoA/src/main.rs", name: "main.rs" },
						{ path: "/repoA/src/lib.rs", name: "lib.rs" },
					],
				}),
			}),
		);
		expect(mockFlushState).toHaveBeenCalledWith("/repoA");
	});

	it("シナリオ2: 切替先のWorktreeの状態が復元される", () => {
		const cachedState = makeState({
			layout: {
				centerTab: "workflow",
				activeView: "git",
				leftNavCollapsed: true,
				rightCollapsed: false,
				rightBottomCollapsed: false,
			},
		});
		mockGetState.mockImplementation((path: string) =>
			path === "/repoB" ? cachedState : undefined,
		);

		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const { result, rerender } = renderHook(
			({ selectedRootPath }) =>
				useWorkspacePersistence({
					selectedRootPath,
					centerTab: "editor",
					leftNavVisible: true,
					rightVisible: true,
					setCenterTab,
					leftNavRef,
					rightPanelRef,
				}),
			{ initialProps: { selectedRootPath: "/repoA" as string | null } },
		);

		act(() => {
			result.current.internalStateMapRef.current.set(
				"/repoA",
				makeInternalState(),
			);
		});

		rerender({ selectedRootPath: "/repoB" });

		// centerTabが同期的に復元される
		expect(setCenterTab).toHaveBeenCalledWith("workflow");

		flushRAF();

		expect(leftNavRef.current.collapse).toHaveBeenCalled();
		expect(rightPanelRef.current.expand).toHaveBeenCalled();

		// タブ一覧・アクティブタブの復元検証
		const initialState = result.current.getInitialState("/repoB");
		expect(initialState).toBeDefined();
		expect(initialState?.tabs.editors).toEqual([
			{ path: "/repoA/src/main.rs", name: "main.rs" },
			{ path: "/repoA/src/lib.rs", name: "lib.rs" },
		]);
		expect(initialState?.tabs.activeEditorPath).toBe("/repoA/src/main.rs");

		// 旧Worktreeのエントリがクリーンアップされていること
		expect(result.current.internalStateMapRef.current.has("/repoA")).toBe(
			false,
		);
	});

	it("シナリオ3: タブの並び順が保持される", () => {
		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const orderedTabs = [
			{ path: "/repoA/src/lib.rs", name: "lib.rs" },
			{ path: "/repoA/src/main.rs", name: "main.rs" },
			{ path: "/repoA/src/utils.rs", name: "utils.rs" },
		];

		const { result, rerender } = renderHook(
			({ selectedRootPath }) =>
				useWorkspacePersistence({
					selectedRootPath,
					centerTab: "editor",
					leftNavVisible: true,
					rightVisible: true,
					setCenterTab,
					leftNavRef,
					rightPanelRef,
				}),
			{ initialProps: { selectedRootPath: "/repoA" as string | null } },
		);

		act(() => {
			result.current.internalStateMapRef.current.set(
				"/repoA",
				makeInternalState({
					tabs: orderedTabs,
				}),
			);
		});

		rerender({ selectedRootPath: "/repoB" });

		const savedState = mockUpdateState.mock.calls[0][1] as WorkspaceState;
		expect(savedState.tabs.editors).toEqual(orderedTabs);
	});

	it("シナリオ4: パネルの表示状態が保持される", () => {
		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const { result, rerender } = renderHook(
			({ selectedRootPath, leftNavVisible, rightVisible }) =>
				useWorkspacePersistence({
					selectedRootPath,
					centerTab: "editor",
					leftNavVisible,
					rightVisible,
					setCenterTab,
					leftNavRef,
					rightPanelRef,
				}),
			{
				initialProps: {
					selectedRootPath: "/repoA" as string | null,
					leftNavVisible: false,
					rightVisible: false,
				},
			},
		);

		act(() => {
			result.current.internalStateMapRef.current.set(
				"/repoA",
				makeInternalState(),
			);
		});

		rerender({
			selectedRootPath: "/repoB",
			leftNavVisible: false,
			rightVisible: false,
		});

		const savedState = mockUpdateState.mock.calls[0][1] as WorkspaceState;
		expect(savedState.layout.leftNavCollapsed).toBe(true);
		expect(savedState.layout.rightCollapsed).toBe(true);

		// Restore with collapsed panels
		const collapsedState = makeState({
			layout: {
				centerTab: "editor",
				activeView: "git",
				leftNavCollapsed: true,
				rightCollapsed: true,
				rightBottomCollapsed: false,
			},
		});
		mockGetState.mockImplementation((path: string) =>
			path === "/repoC" ? collapsedState : undefined,
		);

		rerender({
			selectedRootPath: "/repoC",
			leftNavVisible: false,
			rightVisible: false,
		});

		flushRAF();

		expect(leftNavRef.current.collapse).toHaveBeenCalled();
		expect(rightPanelRef.current.collapse).toHaveBeenCalled();
	});

	it("シナリオ7: 保存された状態が存在しないWorktreeを開く → デフォルトリセット", () => {
		mockGetState.mockReturnValue(undefined);

		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const { rerender } = renderHook(
			({ selectedRootPath }) =>
				useWorkspacePersistence({
					selectedRootPath,
					centerTab: "editor",
					leftNavVisible: true,
					rightVisible: true,
					setCenterTab,
					leftNavRef,
					rightPanelRef,
				}),
			{ initialProps: { selectedRootPath: "/repoA" as string | null } },
		);

		// Switch to a new worktree with no saved state
		rerender({ selectedRootPath: "/newRepo" });

		// centerTabがデフォルトの"workflow"にリセットされる
		expect(setCenterTab).toHaveBeenCalledWith("workflow");

		flushRAF();

		// パネルがexpandされる（デフォルトリセット）
		expect(leftNavRef.current.expand).toHaveBeenCalled();
		expect(rightPanelRef.current.expand).toHaveBeenCalled();
	});

	it("シナリオ8: A→B→A の往復で状態が正しく復元される", () => {
		const stateA = makeState({
			layout: {
				centerTab: "editor",
				activeView: "git",
				leftNavCollapsed: false,
				rightCollapsed: false,
				rightBottomCollapsed: false,
			},
		});
		const stateB = makeState({
			layout: {
				centerTab: "workflow",
				activeView: "explorer",
				leftNavCollapsed: true,
				rightCollapsed: false,
				rightBottomCollapsed: false,
			},
		});

		mockGetState.mockImplementation((path: string) => {
			if (path === "/repoA") return stateA;
			if (path === "/repoB") return stateB;
			return undefined;
		});

		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const { result, rerender } = renderHook(
			({ selectedRootPath }) =>
				useWorkspacePersistence({
					selectedRootPath,
					centerTab: "editor",
					leftNavVisible: true,
					rightVisible: true,
					setCenterTab,
					leftNavRef,
					rightPanelRef,
				}),
			{ initialProps: { selectedRootPath: "/repoA" as string | null } },
		);

		// Set internal state for A
		act(() => {
			result.current.internalStateMapRef.current.set(
				"/repoA",
				makeInternalState(),
			);
		});

		// A → B
		rerender({ selectedRootPath: "/repoB" });
		expect(setCenterTab).toHaveBeenCalledWith("workflow");
		// A's internal state should be cleaned up
		expect(result.current.internalStateMapRef.current.has("/repoA")).toBe(
			false,
		);

		setCenterTab.mockClear();

		// Set internal state for B
		act(() => {
			result.current.internalStateMapRef.current.set(
				"/repoB",
				makeInternalState({ activeView: "explorer" }),
			);
		});

		// B → A (back to A)
		rerender({ selectedRootPath: "/repoA" });

		// B's state should be saved
		expect(mockUpdateState).toHaveBeenCalledWith(
			"/repoB",
			expect.objectContaining({
				version: 1,
			}),
		);
		expect(mockFlushState).toHaveBeenCalledWith("/repoB");

		// A's cached state should be restored
		expect(setCenterTab).toHaveBeenCalledWith("editor");

		// B's internal state should be cleaned up
		expect(result.current.internalStateMapRef.current.has("/repoB")).toBe(
			false,
		);
	});

	it("pre-load: 初回マウント時にloadStateが呼ばれる", () => {
		mockGetState.mockReturnValue(undefined);

		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		renderHook(() =>
			useWorkspacePersistence({
				selectedRootPath: "/repoX",
				centerTab: "editor",
				leftNavVisible: true,
				rightVisible: true,
				setCenterTab,
				leftNavRef,
				rightPanelRef,
			}),
		);

		expect(mockLoadState).toHaveBeenCalledWith("/repoX");
	});

	it("pre-load: キャッシュ済みならloadStateは呼ばれない", () => {
		mockGetState.mockReturnValue(makeState());

		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		renderHook(() =>
			useWorkspacePersistence({
				selectedRootPath: "/repoX",
				centerTab: "editor",
				leftNavVisible: true,
				rightVisible: true,
				setCenterTab,
				leftNavRef,
				rightPanelRef,
			}),
		);

		expect(mockLoadState).not.toHaveBeenCalled();
	});

	it("右下パネルのタブ選択が保存される", () => {
		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const { result, rerender } = renderHook(
			({ selectedRootPath }) =>
				useWorkspacePersistence({
					selectedRootPath,
					centerTab: "editor",
					leftNavVisible: true,
					rightVisible: true,
					setCenterTab,
					leftNavRef,
					rightPanelRef,
				}),
			{ initialProps: { selectedRootPath: "/repoA" as string | null } },
		);

		act(() => {
			result.current.internalStateMapRef.current.set(
				"/repoA",
				makeInternalState({ rightBottomActiveTab: "comment" }),
			);
		});

		rerender({ selectedRootPath: "/repoB" });

		const savedState = mockUpdateState.mock.calls[0][1] as WorkspaceState;
		expect(savedState.layout.rightBottomActiveTab).toBe("comment");
	});

	it("右下パネルのタブ選択がWorktreeごとに独立している", () => {
		const stateA = makeState({
			layout: {
				centerTab: "editor",
				activeView: "git",
				leftNavCollapsed: false,
				rightCollapsed: false,
				rightBottomCollapsed: false,
				rightBottomActiveTab: "comment",
			},
		});
		const stateB = makeState({
			layout: {
				centerTab: "editor",
				activeView: "git",
				leftNavCollapsed: false,
				rightCollapsed: false,
				rightBottomCollapsed: false,
				rightBottomActiveTab: "terminal",
			},
		});

		mockGetState.mockImplementation((path: string) => {
			if (path === "/repoA") return stateA;
			if (path === "/repoB") return stateB;
			return undefined;
		});

		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const { result, rerender } = renderHook(
			({ selectedRootPath }) =>
				useWorkspacePersistence({
					selectedRootPath,
					centerTab: "editor",
					leftNavVisible: true,
					rightVisible: true,
					setCenterTab,
					leftNavRef,
					rightPanelRef,
				}),
			{ initialProps: { selectedRootPath: "/repoA" as string | null } },
		);

		// A → B に切り替え
		act(() => {
			result.current.internalStateMapRef.current.set(
				"/repoA",
				makeInternalState({ rightBottomActiveTab: "comment" }),
			);
		});
		rerender({ selectedRootPath: "/repoB" });

		const initialB = result.current.getInitialState("/repoB");
		expect(initialB?.layout.rightBottomActiveTab).toBe("terminal");
	});

	it("workflowPanelRatios が保存・復元される", () => {
		const stateWithRatios = makeState({
			layout: {
				centerTab: "workflow",
				activeView: "git",
				leftNavCollapsed: false,
				rightCollapsed: false,
				rightBottomCollapsed: false,
				workflowPanelRatios: [65, 35],
			},
		});
		mockGetState.mockImplementation((path: string) =>
			path === "/repoB" ? stateWithRatios : undefined,
		);

		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const { result, rerender } = renderHook(
			({ selectedRootPath }) =>
				useWorkspacePersistence({
					selectedRootPath,
					centerTab: "editor",
					leftNavVisible: true,
					rightVisible: true,
					setCenterTab,
					leftNavRef,
					rightPanelRef,
				}),
			{ initialProps: { selectedRootPath: "/repoA" as string | null } },
		);

		// Set internal state with workflowPanelRatios
		act(() => {
			result.current.internalStateMapRef.current.set(
				"/repoA",
				makeInternalState({ workflowPanelRatios: [70, 30] }),
			);
		});

		// Switch worktree → save
		rerender({ selectedRootPath: "/repoB" });

		const savedState = mockUpdateState.mock.calls[0][1] as WorkspaceState;
		expect(savedState.layout.workflowPanelRatios).toEqual([70, 30]);

		// Restore from /repoB
		const initialState = result.current.getInitialState("/repoB");
		expect(initialState?.layout.workflowPanelRatios).toEqual([65, 35]);
	});

	it("保存された状態が存在しないWorktreeでは右下パネルがデフォルトタブで表示される", () => {
		mockGetState.mockReturnValue(undefined);

		const setCenterTab = vi.fn();
		const leftNavRef = makePanelRef();
		const rightPanelRef = makePanelRef();

		const { result } = renderHook(() =>
			useWorkspacePersistence({
				selectedRootPath: "/newRepo",
				centerTab: "editor",
				leftNavVisible: true,
				rightVisible: true,
				setCenterTab,
				leftNavRef,
				rightPanelRef,
			}),
		);

		const initialState = result.current.getInitialState("/newRepo");
		// 保存状態がないのでundefined、useWorktreeStateでデフォルト"terminal"になる
		expect(initialState).toBeUndefined();
	});
});
