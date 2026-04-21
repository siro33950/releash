import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DiffComment } from "@/types/diffComment";
import { FileCommentPopoverTrigger } from "./DiffFileComment";

const makeComment = (overrides: Partial<DiffComment> = {}): DiffComment => ({
	id: "c1",
	filePath: "src/main.ts",
	lineNumber: undefined,
	endLine: undefined,
	content: "File-level comment",
	status: "unsent",
	createdAt: Date.now(),
	...overrides,
});

describe("FileCommentPopoverTrigger", () => {
	const defaultProps = {
		filePath: "src/main.ts",
		onAdd: vi.fn().mockResolvedValue(undefined),
		onUpdate: vi.fn().mockResolvedValue(undefined),
		onDelete: vi.fn().mockResolvedValue(undefined),
		onSend: vi.fn().mockResolvedValue(undefined),
	};

	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("renders trigger button", () => {
		render(<FileCommentPopoverTrigger comments={[]} {...defaultProps} />);
		expect(screen.getByTitle("File comments")).toBeInTheDocument();
	});

	it("shows badge count when file comments exist", () => {
		const comments = [makeComment({ id: "c1" }), makeComment({ id: "c2" })];
		render(<FileCommentPopoverTrigger comments={comments} {...defaultProps} />);
		expect(screen.getByText("2")).toBeInTheDocument();
	});

	it("does not show badge when no file comments", () => {
		render(<FileCommentPopoverTrigger comments={[]} {...defaultProps} />);
		expect(screen.queryByText("0")).not.toBeInTheDocument();
	});

	it("filters out line comments (only shows file comments)", () => {
		const comments = [
			makeComment({ id: "c1", lineNumber: undefined }),
			makeComment({ id: "c2", lineNumber: 10 }),
		];
		render(<FileCommentPopoverTrigger comments={comments} {...defaultProps} />);
		expect(screen.getByText("1")).toBeInTheDocument();
	});

	it("filters by filePath", () => {
		const comments = [
			makeComment({ id: "c1", filePath: "src/main.ts" }),
			makeComment({ id: "c2", filePath: "src/other.ts" }),
		];
		render(<FileCommentPopoverTrigger comments={comments} {...defaultProps} />);
		expect(screen.getByText("1")).toBeInTheDocument();
	});

	it("opens popover and shows comments on click", async () => {
		const user = userEvent.setup();
		const comments = [makeComment({ content: "Review this file" })];
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
