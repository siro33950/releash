import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Thread } from "@/types/thread";
import { useWorkflowState } from "./useWorkflowState";

const mockThreads: Thread[] = [
	{
		id: "t1",
		filePath: "workflow://plan",
		lineNumber: 1,
		entries: [
			{
				id: "e1",
				content: "test",
				isAi: false,
				createdAt: Date.now(),
			},
		],
		resolved: false,
		createdAt: Date.now(),
	},
];

const mockCreateThread = vi.fn().mockResolvedValue({ id: "t-new" });
const mockAddEntry = vi.fn();
const mockRemoveThread = vi.fn();
const mockUpdateEntry = vi.fn();
const mockResolveThread = vi.fn();
const mockRecalculateAnchors = vi.fn();
const mockToggleShowResolved = vi.fn();

vi.mock("@/hooks/useThreads", () => ({
	useThreads: vi.fn((worktreeName: string) => ({
		threads: worktreeName.endsWith("::plan") ? mockThreads : [],
		createThread: mockCreateThread,
		addEntry: mockAddEntry,
		removeThread: mockRemoveThread,
		updateEntry: mockUpdateEntry,
		resolveThread: mockResolveThread,
		recalculateAnchorsForFile: mockRecalculateAnchors,
		showResolvedThreads: false,
		toggleShowResolvedThreads: mockToggleShowResolved,
		unresolvedThreads: [],
		getThreadsForFile: vi.fn(() => []),
		setThreads: vi.fn(),
	})),
}));

const baseParams = {
	rootPath: "/repo",
	getFileContent: vi.fn(),
	updateContent: vi.fn(),
	saveFile: vi.fn().mockResolvedValue(undefined),
	registerVirtualFile: vi.fn(),
	theme: "dark" as const,
	fontSize: 14,
};

describe("useWorkflowState", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("マウント時に registerVirtualFile で plan ドキュメントを登録する", () => {
		const registerVirtualFile = vi.fn();
		renderHook(() => useWorkflowState({ ...baseParams, registerVirtualFile }));
		expect(registerVirtualFile).toHaveBeenCalledWith(
			"workflow://plan",
			"",
			"markdown",
		);
	});

	it("planThreads が useThreads 経由で返される", () => {
		const { result } = renderHook(() => useWorkflowState(baseParams));
		expect(result.current.planThreads).toEqual(mockThreads);
	});

	it("workflowPanelRatios の初期値と更新", () => {
		const { result } = renderHook(() =>
			useWorkflowState({ ...baseParams, initialPanelRatios: [60, 40] }),
		);
		expect(result.current.workflowPanelRatios).toEqual([60, 40]);

		act(() => result.current.setWorkflowPanelRatios([70, 30]));
		expect(result.current.workflowPanelRatios).toEqual([70, 30]);
	});

	it("workflowPanelRatios が initialPanelRatios 未指定時に undefined", () => {
		const { result } = renderHook(() => useWorkflowState(baseParams));
		expect(result.current.workflowPanelRatios).toBeUndefined();
	});

	describe("planEditorContextValue", () => {
		function getPlanCtx() {
			const { result } = renderHook(() => useWorkflowState(baseParams));
			return result.current.planEditorContextValue;
		}

		it("必要なファイル操作プロパティが渡されている", () => {
			const ctx = getPlanCtx();
			expect(ctx.getFileContent).toBe(baseParams.getFileContent);
			expect(ctx.updateContent).toBe(baseParams.updateContent);
			expect(ctx.saveFile).toBe(baseParams.saveFile);
			expect(ctx.rootPath).toBe("/repo");
		});

		it("テーマ・フォントサイズが渡されている", () => {
			const ctx = getPlanCtx();
			expect(ctx.theme).toBe("dark");
			expect(ctx.fontSize).toBe(14);
		});

		it("plan threads が設定されている", () => {
			const ctx = getPlanCtx();
			expect(ctx.threads).toEqual(mockThreads);
		});

		it("Git/diff プロパティがデフォルト値になっている", () => {
			const ctx = getPlanCtx();
			expect(ctx.diffBase).toBe("staged");
			expect(ctx.diffMode).toBe("inline");
			expect(ctx.gitRefreshKey).toBe(0);

			// setDiffBase / setDiffMode は no-op
			ctx.setDiffBase("branch-base");
			ctx.setDiffMode("split");
			expect(ctx.diffBase).toBe("staged");
			expect(ctx.diffMode).toBe("inline");
		});

		it("LSP プロパティがデフォルト値になっている", () => {
			const ctx = getPlanCtx();
			expect(ctx.lspStatus).toBe("idle");
			expect(ctx.lspError).toBeNull();
			expect(ctx.lspCrashCount).toBe(0);
			// lspRetryManually は no-op (呼んでもエラーにならない)
			expect(() => ctx.lspRetryManually()).not.toThrow();
		});

		it("仮想ファイルに不要なオプショナルプロパティが省略されている", () => {
			const ctx = getPlanCtx();
			expect(ctx.onStageHunk).toBeUndefined();
			expect(ctx.onGitChanged).toBeUndefined();
			expect(ctx.implementThread).toBeUndefined();
			expect(ctx.onPostToPr).toBeUndefined();
			expect(ctx.sendThread).toBeUndefined();
			expect(ctx.copyThread).toBeUndefined();
			expect(ctx.onSearchOccurrences).toBeUndefined();
			expect(ctx.onAskAI).toBeUndefined();
			expect(ctx.aiRunningThreadIds).toBeUndefined();
			expect(ctx.aiTaskThreadIds).toBeUndefined();
			expect(ctx.onOpenThreadAIModal).toBeUndefined();
		});

		it("thread 操作が plan 用の関数に接続されている", () => {
			const ctx = getPlanCtx();
			ctx.addEntry("t1", "hello");
			expect(mockAddEntry).toHaveBeenCalledWith("t1", "hello");

			ctx.deleteThread("t1");
			expect(mockRemoveThread).toHaveBeenCalledWith("t1");
		});

		it("createThread が plan 用の関数に接続されている", async () => {
			const { result } = renderHook(() => useWorkflowState(baseParams));
			await result.current.planEditorContextValue.createThread(
				"workflow://plan",
				5,
				"test comment",
				undefined,
				undefined,
			);
			expect(mockCreateThread).toHaveBeenCalledWith(
				"workflow://plan",
				5,
				"test comment",
				undefined,
				undefined,
				undefined,
				undefined,
				undefined,
			);
		});

		it("recalculateAnchorsForFile が plan 用の関数に接続されている", () => {
			const { result } = renderHook(() => useWorkflowState(baseParams));
			result.current.planEditorContextValue.recalculateAnchorsForFile?.(
				"workflow://plan",
				"content",
			);
			expect(mockRecalculateAnchors).toHaveBeenCalledWith(
				"workflow://plan",
				"content",
			);
		});

		it("参照等価性が依存値未変更時に保持される", () => {
			const { result, rerender } = renderHook(() =>
				useWorkflowState(baseParams),
			);
			const first = result.current.planEditorContextValue;
			rerender();
			expect(result.current.planEditorContextValue).toBe(first);
		});
	});
});
