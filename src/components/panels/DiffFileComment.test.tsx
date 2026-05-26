import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReviewDiscussionThread } from "@/types/diffComment";
import { FileCommentPopoverTrigger } from "./DiffFileComment";

const makeComment = (
	overrides: Partial<ReviewDiscussionThread> = {},
): ReviewDiscussionThread => ({
	id: "t1",
	worktreeName: "wt",
	author: { kind: "human", displayName: "Human" },
	target: { filePath: "src/main.ts", lineNumber: null, endLine: null },
	state: "open",
	comments: [
		{
			id: "c1",
			threadId: "t1",
			author: { kind: "human", displayName: "Human" },
			content: "File-level comment",
			createdAt: Date.now(),
		},
	],
	stances: [],
	resolve: null,
	createdAt: Date.now(),
	updatedAt: Date.now(),
	version: 1,
	canResolve: true,
	myStance: "none",
	...overrides,
});

describe("FileCommentPopoverTrigger", () => {
	const defaultProps = {
		filePath: "src/main.ts",
		onAdd: vi.fn().mockResolvedValue(undefined),
		onAppend: vi.fn().mockResolvedValue(undefined),
		onSetStance: vi.fn().mockResolvedValue(undefined),
		onResolve: vi.fn().mockResolvedValue(undefined),
	};

	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("renders trigger button", () => {
		render(<FileCommentPopoverTrigger comments={[]} {...defaultProps} />);
		expect(screen.getByTitle("File comments")).toBeInTheDocument();
	});

	it("shows badge count when file comments exist", () => {
		const comments = [makeComment({ id: "t1" }), makeComment({ id: "t2" })];
		render(<FileCommentPopoverTrigger comments={comments} {...defaultProps} />);
		expect(screen.getByText("2")).toBeInTheDocument();
	});

	it("filters out line comments and other files", () => {
		const comments = [
			makeComment({ id: "t1" }),
			makeComment({
				id: "t2",
				target: { filePath: "src/main.ts", lineNumber: 10, endLine: null },
			}),
			makeComment({
				id: "t3",
				target: { filePath: "src/other.ts", lineNumber: null, endLine: null },
			}),
		];
		render(<FileCommentPopoverTrigger comments={comments} {...defaultProps} />);
		expect(screen.getByText("1")).toBeInTheDocument();
	});

	it("opens popover and shows comments on click", async () => {
		const user = userEvent.setup();
		const comments = [
			makeComment({
				comments: [
					{
						id: "c1",
						threadId: "t1",
						author: { kind: "human", displayName: "Human" },
						content: "Review this file",
						createdAt: Date.now(),
					},
				],
			}),
		];
		render(<FileCommentPopoverTrigger comments={comments} {...defaultProps} />);

		await user.click(screen.getByTitle("File comments"));
		expect(screen.getByText("Review this file")).toBeInTheDocument();
	});

	it("shows add comment button in popover", async () => {
		const user = userEvent.setup();
		render(<FileCommentPopoverTrigger comments={[]} {...defaultProps} />);

		await user.click(screen.getByTitle("File comments"));
		expect(screen.getByText("Add file comment")).toBeInTheDocument();
	});
});
