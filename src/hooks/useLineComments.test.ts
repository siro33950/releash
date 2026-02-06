import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useLineComments } from "./useLineComments";

describe("useLineComments", () => {
	it("should start with empty comments", () => {
		const { result } = renderHook(() => useLineComments());
		expect(result.current.comments).toEqual([]);
		expect(result.current.unsentComments).toEqual([]);
	});

	it("should add a comment", () => {
		const { result } = renderHook(() => useLineComments());

		act(() => {
			result.current.addComment("src/file.ts", 10, "Fix this");
		});

		expect(result.current.comments.length).toBe(1);
		expect(result.current.comments[0].filePath).toBe("src/file.ts");
		expect(result.current.comments[0].lineNumber).toBe(10);
		expect(result.current.comments[0].content).toBe("Fix this");
		expect(result.current.comments[0].status).toBe("unsent");
	});

	it("should remove a comment", () => {
		const { result } = renderHook(() => useLineComments());

		let commentId: string;
		act(() => {
			const comment = result.current.addComment(
				"src/file.ts",
				10,
				"Fix this",
			);
			commentId = comment.id;
		});

		act(() => {
			result.current.removeComment(commentId);
		});

		expect(result.current.comments.length).toBe(0);
	});

	it("should update a comment", () => {
		const { result } = renderHook(() => useLineComments());

		let commentId: string;
		act(() => {
			const comment = result.current.addComment(
				"src/file.ts",
				10,
				"Original",
			);
			commentId = comment.id;
		});

		act(() => {
			result.current.updateComment(commentId, "Updated");
		});

		expect(result.current.comments[0].content).toBe("Updated");
	});

	it("should mark comments as sent", () => {
		const { result } = renderHook(() => useLineComments());

		let id1: string;
		let id2: string;
		act(() => {
			const c1 = result.current.addComment("a.ts", 1, "comment1");
			const c2 = result.current.addComment("b.ts", 2, "comment2");
			id1 = c1.id;
			id2 = c2.id;
		});

		act(() => {
			result.current.markAsSent([id1]);
		});

		expect(
			result.current.comments.find((c) => c.id === id1)?.status,
		).toBe("sent");
		expect(
			result.current.comments.find((c) => c.id === id2)?.status,
		).toBe("unsent");
	});

	it("should return unsent comments", () => {
		const { result } = renderHook(() => useLineComments());

		let id1: string;
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

	it("should add a multi-line comment with endLine", () => {
		const { result } = renderHook(() => useLineComments());

		act(() => {
			result.current.addComment("src/file.ts", 5, "Range comment", 12);
		});

		expect(result.current.comments.length).toBe(1);
		expect(result.current.comments[0].lineNumber).toBe(5);
		expect(result.current.comments[0].endLine).toBe(12);
		expect(result.current.comments[0].content).toBe("Range comment");
	});

	it("should not set endLine when not provided", () => {
		const { result } = renderHook(() => useLineComments());

		act(() => {
			result.current.addComment("src/file.ts", 10, "Single line");
		});

		expect(result.current.comments[0].endLine).toBeUndefined();
	});

	it("should filter comments by file path", () => {
		const { result } = renderHook(() => useLineComments());

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
});
