import { describe, expect, it } from "vitest";
import type { PrReviewComment } from "./prCommentMapping";
import { prReviewCommentsToThreads } from "./prCommentMapping";

function makeComment(
	overrides: Partial<PrReviewComment> = {},
): PrReviewComment {
	return {
		id: 1,
		path: "src/main.rs",
		line: 10,
		original_line: null,
		body: "test comment",
		author: { login: "reviewer", avatar_url: null },
		in_reply_to_id: null,
		created_at: "2024-01-01T00:00:00Z",
		...overrides,
	};
}

describe("prReviewCommentsToThreads", () => {
	it("should convert a single root comment to a thread", () => {
		const threads = prReviewCommentsToThreads([makeComment()]);
		expect(threads).toHaveLength(1);
		expect(threads[0].id).toBe("pr-comment-1");
		expect(threads[0].filePath).toBe("src/main.rs");
		expect(threads[0].lineNumber).toBe(10);
		expect(threads[0].entries).toHaveLength(1);
		expect(threads[0].entries[0].content).toBe("test comment");
		expect(threads[0].entries[0].prCommentId).toBe(1);
		expect(threads[0].entries[0].authorName).toBe("reviewer");
	});

	it("should group replies under root comment", () => {
		const comments = [
			makeComment({ id: 1, body: "root" }),
			makeComment({
				id: 2,
				body: "reply",
				in_reply_to_id: 1,
				created_at: "2024-01-01T01:00:00Z",
			}),
		];
		const threads = prReviewCommentsToThreads(comments);
		expect(threads).toHaveLength(1);
		expect(threads[0].entries).toHaveLength(2);
		expect(threads[0].entries[0].content).toBe("root");
		expect(threads[0].entries[1].content).toBe("reply");
	});

	it("should sort replies by created_at", () => {
		const comments = [
			makeComment({ id: 1, body: "root" }),
			makeComment({
				id: 3,
				body: "later",
				in_reply_to_id: 1,
				created_at: "2024-01-01T03:00:00Z",
			}),
			makeComment({
				id: 2,
				body: "earlier",
				in_reply_to_id: 1,
				created_at: "2024-01-01T01:00:00Z",
			}),
		];
		const threads = prReviewCommentsToThreads(comments);
		expect(threads[0].entries[1].content).toBe("earlier");
		expect(threads[0].entries[2].content).toBe("later");
	});

	it("should use original_line when line is null", () => {
		const threads = prReviewCommentsToThreads([
			makeComment({ line: null, original_line: 25 }),
		]);
		expect(threads[0].lineNumber).toBe(25);
	});

	it("should default lineNumber to 1 when both line and original_line are null", () => {
		const threads = prReviewCommentsToThreads([
			makeComment({ line: null, original_line: null }),
		]);
		expect(threads[0].lineNumber).toBe(1);
	});

	it("should handle empty comments", () => {
		const threads = prReviewCommentsToThreads([]);
		expect(threads).toHaveLength(0);
	});

	it("should create separate threads for different root comments", () => {
		const comments = [
			makeComment({ id: 1, path: "a.rs", line: 10, body: "first" }),
			makeComment({ id: 2, path: "b.rs", line: 20, body: "second" }),
		];
		const threads = prReviewCommentsToThreads(comments);
		expect(threads).toHaveLength(2);
		expect(threads[0].filePath).toBe("a.rs");
		expect(threads[1].filePath).toBe("b.rs");
	});

	it("should preserve avatar_url in entries", () => {
		const threads = prReviewCommentsToThreads([
			makeComment({
				author: {
					login: "user1",
					avatar_url: "https://avatars.example.com/1",
				},
			}),
		]);
		expect(threads[0].entries[0].authorAvatarUrl).toBe(
			"https://avatars.example.com/1",
		);
	});

	it("should set prCommentId on all entries", () => {
		const comments = [
			makeComment({ id: 100, body: "root" }),
			makeComment({ id: 200, body: "reply", in_reply_to_id: 100 }),
		];
		const threads = prReviewCommentsToThreads(comments);
		expect(threads[0].entries[0].prCommentId).toBe(100);
		expect(threads[0].entries[1].prCommentId).toBe(200);
	});
});
