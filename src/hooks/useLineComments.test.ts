import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useLineComments } from "./useLineComments";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue([]),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(() => {}),
}));

const { invoke } = await import("@tauri-apps/api/core");
const mockedInvoke = vi.mocked(invoke);

const WORKTREE = "/tmp/test-wt";

describe("useLineComments", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockedInvoke.mockResolvedValue([]);
	});

	it("should load comments on mount via invoke", async () => {
		mockedInvoke.mockResolvedValueOnce([
			{
				id: "abc-123",
				file_path: "src/file.ts",
				line_number: 10,
				content: "Loaded",
				status: "unsent",
				created_at: 1000,
				author: { type: "human", name: "User" },
				resolved: false,
				target: "local",
			},
		]);

		const { result } = renderHook(() => useLineComments(WORKTREE));

		await act(async () => {});

		expect(mockedInvoke).toHaveBeenCalledWith("load_comments", {
			worktreeName: WORKTREE,
		});
		expect(result.current.comments.length).toBe(1);
		expect(result.current.comments[0].filePath).toBe("src/file.ts");
	});

	it("should start with empty comments before load resolves", () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		expect(result.current.comments).toEqual([]);
		expect(result.current.unsentComments).toEqual([]);
	});

	it("should add a comment with UUID id and invoke backend", async () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		await act(async () => {});

		act(() => {
			result.current.addComment("src/file.ts", 10, "Fix this");
		});

		expect(result.current.comments.length).toBe(1);
		const comment = result.current.comments[0];
		expect(comment.filePath).toBe("src/file.ts");
		expect(comment.lineNumber).toBe(10);
		expect(comment.content).toBe("Fix this");
		expect(comment.status).toBe("unsent");
		// UUID format check
		expect(comment.id).toMatch(
			/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
		);
		expect(mockedInvoke).toHaveBeenCalledWith(
			"add_comment",
			expect.objectContaining({
				worktreeName: WORKTREE,
				source: "desktop",
			}),
		);
	});

	it("should remove a comment and invoke backend", async () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		await act(async () => {});

		let commentId = "";
		act(() => {
			const comment = result.current.addComment("src/file.ts", 10, "Fix this");
			commentId = comment.id;
		});

		act(() => {
			result.current.removeComment(commentId);
		});

		expect(result.current.comments.length).toBe(0);
		expect(mockedInvoke).toHaveBeenCalledWith("remove_comment", {
			worktreeName: WORKTREE,
			id: commentId,
			source: "desktop",
		});
	});

	it("should update a comment and invoke backend", async () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		await act(async () => {});

		let commentId = "";
		act(() => {
			const comment = result.current.addComment("src/file.ts", 10, "Original");
			commentId = comment.id;
		});

		act(() => {
			result.current.updateComment(commentId, "Updated");
		});

		expect(result.current.comments[0].content).toBe("Updated");
		expect(mockedInvoke).toHaveBeenCalledWith("update_comment_content", {
			worktreeName: WORKTREE,
			id: commentId,
			content: "Updated",
			source: "desktop",
		});
	});

	it("should mark comments as sent and invoke backend", async () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		await act(async () => {});

		let id1 = "";
		let id2 = "";
		act(() => {
			const c1 = result.current.addComment("a.ts", 1, "comment1");
			const c2 = result.current.addComment("b.ts", 2, "comment2");
			id1 = c1.id;
			id2 = c2.id;
		});

		act(() => {
			result.current.markAsSent([id1]);
		});

		expect(result.current.comments.find((c) => c.id === id1)?.status).toBe(
			"sent",
		);
		expect(result.current.comments.find((c) => c.id === id2)?.status).toBe(
			"unsent",
		);
		expect(mockedInvoke).toHaveBeenCalledWith("mark_comments_sent", {
			worktreeName: WORKTREE,
			ids: [id1],
			source: "desktop",
		});
	});

	it("should resolve a comment and invoke backend", async () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		await act(async () => {});

		let commentId = "";
		act(() => {
			const c = result.current.addComment("a.ts", 1, "comment");
			commentId = c.id;
		});

		act(() => {
			result.current.resolveComment(commentId);
		});

		expect(
			result.current.comments.find((c) => c.id === commentId)?.resolved,
		).toBe(true);
		expect(mockedInvoke).toHaveBeenCalledWith("toggle_resolve_comment", {
			worktreeName: WORKTREE,
			id: commentId,
			source: "desktop",
		});
	});

	it("should return unsent comments", async () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		await act(async () => {});

		let id1 = "";
		act(() => {
			const c1 = result.current.addComment("a.ts", 1, "comment1");
			result.current.addComment("b.ts", 2, "comment2");
			id1 = c1.id;
		});

		act(() => {
			result.current.markAsSent([id1]);
		});

		expect(result.current.unsentComments.length).toBe(1);
		expect(result.current.unsentComments[0].content).toBe("comment2");
	});

	it("should add a multi-line comment with endLine", async () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		await act(async () => {});

		act(() => {
			result.current.addComment("src/file.ts", 5, "Range comment", 12);
		});

		expect(result.current.comments.length).toBe(1);
		expect(result.current.comments[0].lineNumber).toBe(5);
		expect(result.current.comments[0].endLine).toBe(12);
	});

	it("should not set endLine when not provided", async () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		await act(async () => {});

		act(() => {
			result.current.addComment("src/file.ts", 10, "Single line");
		});

		expect(result.current.comments[0].endLine).toBeUndefined();
	});

	it("should filter comments by file path", async () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		await act(async () => {});

		act(() => {
			result.current.addComment("a.ts", 1, "comment1");
			result.current.addComment("b.ts", 2, "comment2");
			result.current.addComment("a.ts", 3, "comment3");
		});

		const aComments = result.current.getCommentsForFile("a.ts");
		expect(aComments.length).toBe(2);
		expect(aComments[0].content).toBe("comment1");
		expect(aComments[1].content).toBe("comment3");
	});

	it("should default showSentComments to false", () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));
		expect(result.current.showSentComments).toBe(false);
	});

	it("should toggle showSentComments", () => {
		const { result } = renderHook(() => useLineComments(WORKTREE));

		expect(result.current.showSentComments).toBe(false);

		act(() => {
			result.current.toggleShowSentComments();
		});

		expect(result.current.showSentComments).toBe(true);

		act(() => {
			result.current.toggleShowSentComments();
		});

		expect(result.current.showSentComments).toBe(false);
	});
});
