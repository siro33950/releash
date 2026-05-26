import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReviewDiscussionThread } from "@/types/diffComment";
import { DiffInlineComment, DiffInlineCommentInput } from "./DiffInlineComment";

const makeComment = (
	overrides: Partial<ReviewDiscussionThread> = {},
): ReviewDiscussionThread => ({
	id: "c1",
	worktreeName: "wt",
	author: {
		kind: "human",
		displayName: "Human",
		backendId: null,
		model: null,
	},
	target: { filePath: "src/main.ts", lineNumber: 10, endLine: null },
	state: "open",
	comments: [
		{
			id: "comment-1",
			threadId: "c1",
			author: { kind: "human", displayName: "Human" },
			content: "Fix this bug",
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

describe("DiffInlineComment", () => {
	const defaultProps = {
		onAppend: vi.fn().mockResolvedValue(undefined),
		onSetStance: vi.fn().mockResolvedValue(undefined),
		onResolve: vi.fn().mockResolvedValue(undefined),
	};

	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("renders comment content", () => {
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);
		expect(screen.getByText("Fix this bug")).toBeInTheDocument();
	});

	it("renders line label for single line comment", () => {
		render(
			<DiffInlineComment
				comment={makeComment({
					target: { filePath: "src/main.ts", lineNumber: 42, endLine: null },
				})}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("L42")).toBeInTheDocument();
	});

	it("renders range label for multi-line comment", () => {
		render(
			<DiffInlineComment
				comment={makeComment({
					target: { filePath: "src/main.ts", lineNumber: 10, endLine: 20 },
				})}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("L10-20")).toBeInTheDocument();
	});

	it("does not render line label for file comment", () => {
		render(
			<DiffInlineComment
				comment={makeComment({
					target: { filePath: "src/main.ts", lineNumber: null, endLine: null },
				})}
				{...defaultProps}
			/>,
		);
		expect(screen.queryByText(/^L\d/)).not.toBeInTheDocument();
	});

	it("shows resolved badge when status is resolved", () => {
		render(
			<DiffInlineComment
				comment={makeComment({ state: "resolved" })}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("resolved")).toBeInTheDocument();
	});

	it("renders actor kind, all stances, resolve metadata, and projected my stance", () => {
		render(
			<DiffInlineComment
				comment={makeComment({
					myStance: "agree",
					stances: [
						{
							actor: { kind: "human", displayName: "Human" },
							value: "agree",
							updatedAt: 1,
						},
						{
							actor: { kind: "agent", displayName: "codex/gpt-5" },
							value: "disagree",
							updatedAt: 2,
						},
					],
					resolve: {
						actor: { kind: "human", displayName: "Human" },
						outcome: "resolved",
						summary: "Fixed",
						resolvedAt: 3,
					},
				})}
				{...defaultProps}
			/>,
		);

		expect(screen.getByText("human · Human")).toBeInTheDocument();
		expect(screen.getByText("codex/gpt-5")).toBeInTheDocument();
		expect(screen.getAllByText("disagree").length).toBeGreaterThan(0);
		expect(screen.getByRole("button", { name: "agree" })).toHaveClass(
			"bg-primary",
		);
		expect(screen.getByText("resolved by human · Human")).toBeInTheDocument();
		expect(screen.getByText("Fixed")).toBeInTheDocument();
	});

	it("does not render legacy edit, delete, or send controls", () => {
		render(
			<DiffInlineComment
				comment={makeComment({ state: "open" })}
				{...defaultProps}
			/>,
		);
		expect(screen.queryByTitle("Edit")).not.toBeInTheDocument();
		expect(screen.queryByTitle("Delete")).not.toBeInTheDocument();
		expect(screen.queryByTitle("Send to Agent")).not.toBeInTheDocument();
	});

	it("wires stance, reply, and resolve actions to thread operations", async () => {
		const user = userEvent.setup();
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);

		await user.click(screen.getByRole("button", { name: "agree" }));
		await user.type(screen.getByPlaceholderText("Reply..."), "Looks fixed");
		await user.click(screen.getByRole("button", { name: "Reply" }));
		await user.type(
			screen.getByPlaceholderText("Resolution summary"),
			"Resolved by change",
		);
		await user.click(screen.getByTitle("Resolve"));

		expect(defaultProps.onSetStance).toHaveBeenCalledWith("c1", "agree");
		expect(defaultProps.onAppend).toHaveBeenCalledWith("c1", "Looks fixed");
		expect(defaultProps.onResolve).toHaveBeenCalledWith(
			"c1",
			"resolved",
			"Resolved by change",
		);
	});
});

describe("DiffInlineCommentInput", () => {
	const defaultProps = {
		onSubmit: vi.fn().mockResolvedValue(undefined),
		onCancel: vi.fn(),
	};

	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("renders textarea with placeholder", () => {
		render(<DiffInlineCommentInput {...defaultProps} />);
		expect(
			screen.getByPlaceholderText("Leave a comment..."),
		).toBeInTheDocument();
	});

	it("renders range label when provided", () => {
		render(<DiffInlineCommentInput {...defaultProps} rangeLabel="L10-L20" />);
		expect(screen.getByText("L10-L20")).toBeInTheDocument();
	});

	it("submits content and clears input", async () => {
		const user = userEvent.setup();
		render(<DiffInlineCommentInput {...defaultProps} />);

		const textarea = screen.getByPlaceholderText("Leave a comment...");
		await user.type(textarea, "New comment");
		await user.click(screen.getByText("Comment"));

		expect(defaultProps.onSubmit).toHaveBeenCalledWith("New comment");
	});

	it("disables submit button when content is empty", () => {
		render(<DiffInlineCommentInput {...defaultProps} />);
		expect(screen.getByText("Comment")).toBeDisabled();
	});

	it("calls onCancel when cancel button clicked", async () => {
		const user = userEvent.setup();
		render(<DiffInlineCommentInput {...defaultProps} />);

		await user.click(screen.getByText("Cancel"));
		expect(defaultProps.onCancel).toHaveBeenCalled();
	});

	it("calls onCancel on Escape key", () => {
		render(<DiffInlineCommentInput {...defaultProps} />);

		const textarea = screen.getByPlaceholderText("Leave a comment...");
		fireEvent.keyDown(textarea, { key: "Escape" });
		expect(defaultProps.onCancel).toHaveBeenCalled();
	});
});
