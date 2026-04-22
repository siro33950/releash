import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DiffComment } from "@/types/diffComment";
import { DiffInlineComment, DiffInlineCommentInput } from "./DiffInlineComment";

const makeComment = (overrides: Partial<DiffComment> = {}): DiffComment => ({
	id: "c1",
	filePath: "src/main.ts",
	lineNumber: 10,
	endLine: undefined,
	content: "Fix this bug",
	status: "unsent",
	createdAt: Date.now(),
	...overrides,
});

describe("DiffInlineComment", () => {
	const defaultProps = {
		onUpdate: vi.fn().mockResolvedValue(undefined),
		onDelete: vi.fn().mockResolvedValue(undefined),
		onSend: vi.fn().mockResolvedValue(undefined),
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
				comment={makeComment({ lineNumber: 42 })}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("L42")).toBeInTheDocument();
	});

	it("renders range label for multi-line comment", () => {
		render(
			<DiffInlineComment
				comment={makeComment({ lineNumber: 10, endLine: 20 })}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("L10-20")).toBeInTheDocument();
	});

	it("does not render line label for file comment", () => {
		render(
			<DiffInlineComment
				comment={makeComment({ lineNumber: undefined })}
				{...defaultProps}
			/>,
		);
		expect(screen.queryByText(/^L\d/)).not.toBeInTheDocument();
	});

	it("shows sent badge when status is sent", () => {
		render(
			<DiffInlineComment
				comment={makeComment({ status: "sent" })}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("sent")).toBeInTheDocument();
	});

	it("shows send button when status is unsent", () => {
		render(
			<DiffInlineComment
				comment={makeComment({ status: "unsent" })}
				{...defaultProps}
			/>,
		);
		expect(screen.getByTitle("Send to Agent")).toBeInTheDocument();
	});

	it("hides send button when status is sent", () => {
		render(
			<DiffInlineComment
				comment={makeComment({ status: "sent" })}
				{...defaultProps}
			/>,
		);
		expect(screen.queryByTitle("Send to Agent")).not.toBeInTheDocument();
	});

	it("calls onDelete when delete button clicked", async () => {
		const user = userEvent.setup();
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);

		await user.click(screen.getByTitle("Delete"));
		expect(defaultProps.onDelete).toHaveBeenCalledWith("c1");
	});

	it("calls onSend when send button clicked", async () => {
		const user = userEvent.setup();
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);

		await user.click(screen.getByTitle("Send to Agent"));
		expect(defaultProps.onSend).toHaveBeenCalledWith(["c1"]);
	});

	it("enters edit mode when edit button clicked", async () => {
		const user = userEvent.setup();
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);

		await user.click(screen.getByTitle("Edit"));
		expect(screen.getByRole("textbox")).toBeInTheDocument();
		expect(screen.getByRole("textbox")).toHaveValue("Fix this bug");
	});

	it("saves edited content", async () => {
		const user = userEvent.setup();
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);

		await user.click(screen.getByTitle("Edit"));
		const textarea = screen.getByRole("textbox");
		await user.clear(textarea);
		await user.type(textarea, "Updated content");
		await user.click(screen.getByText("Save"));

		expect(defaultProps.onUpdate).toHaveBeenCalledWith("c1", "Updated content");
	});

	it("cancels editing and restores original content", async () => {
		const user = userEvent.setup();
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);

		await user.click(screen.getByTitle("Edit"));
		const textarea = screen.getByRole("textbox");
		await user.clear(textarea);
		await user.type(textarea, "Changed");
		await user.click(screen.getByText("Cancel"));

		expect(screen.getByText("Fix this bug")).toBeInTheDocument();
		expect(defaultProps.onUpdate).not.toHaveBeenCalled();
	});

	it("does not save empty content", async () => {
		const user = userEvent.setup();
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);

		await user.click(screen.getByTitle("Edit"));
		const textarea = screen.getByRole("textbox");
		await user.clear(textarea);

		const saveButton = screen.getByText("Save");
		expect(saveButton).toBeDisabled();
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
