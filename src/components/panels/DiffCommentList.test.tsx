import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { DiffComment } from "@/types/diffComment";
import { DiffCommentList } from "./DiffCommentList";

function renderWithProviders(ui: React.ReactElement) {
	return render(<TooltipProvider>{ui}</TooltipProvider>);
}

const makeComment = (overrides: Partial<DiffComment> = {}): DiffComment => ({
	id: "c1",
	filePath: "src/main.ts",
	lineNumber: 10,
	endLine: undefined,
	content: "Fix this",
	status: "unsent",
	createdAt: Date.now(),
	...overrides,
});

describe("DiffCommentList", () => {
	const defaultProps = {
		unsentCount: 0,
		onCommentClick: vi.fn(),
		onDelete: vi.fn().mockResolvedValue(undefined),
		onSend: vi.fn().mockResolvedValue(undefined),
		onSendAll: vi.fn().mockResolvedValue(undefined),
	};

	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("shows empty state when no comments", () => {
		renderWithProviders(<DiffCommentList comments={[]} {...defaultProps} />);
		expect(screen.getByText("No comments yet")).toBeInTheDocument();
		expect(screen.getByText("Add comments on diff lines")).toBeInTheDocument();
	});

	it("renders header with Comments label", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment()]}
				{...defaultProps}
				unsentCount={1}
			/>,
		);
		expect(screen.getByText("Comments")).toBeInTheDocument();
	});

	it("shows unsent count badge", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment()]}
				{...defaultProps}
				unsentCount={3}
			/>,
		);
		expect(screen.getByText("3")).toBeInTheDocument();
	});

	it("does not show badge when unsentCount is 0", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ status: "sent" })]}
				{...defaultProps}
				unsentCount={0}
			/>,
		);
		const badges = screen.queryAllByText("0");
		expect(badges).toHaveLength(0);
	});

	it("groups comments by file", () => {
		const comments = [
			makeComment({ id: "c1", filePath: "src/a.ts", content: "Comment A" }),
			makeComment({ id: "c2", filePath: "src/b.ts", content: "Comment B" }),
			makeComment({ id: "c3", filePath: "src/a.ts", content: "Comment C" }),
		];
		renderWithProviders(
			<DiffCommentList comments={comments} {...defaultProps} />,
		);
		expect(screen.getByText("a.ts")).toBeInTheDocument();
		expect(screen.getByText("b.ts")).toBeInTheDocument();
		expect(screen.getByText("Comment A")).toBeInTheDocument();
		expect(screen.getByText("Comment B")).toBeInTheDocument();
		expect(screen.getByText("Comment C")).toBeInTheDocument();
	});

	it("shows line label for line comments", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ lineNumber: 42 })]}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("L42")).toBeInTheDocument();
	});

	it("shows range label for multi-line comments", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ lineNumber: 10, endLine: 20 })]}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("L10-20")).toBeInTheDocument();
	});

	it("shows 'file' label for file comments", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ lineNumber: undefined })]}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("file")).toBeInTheDocument();
	});

	it("shows sent badge for sent comments", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ status: "sent" })]}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("sent")).toBeInTheDocument();
	});

	it("calls onCommentClick with filePath and lineNumber", async () => {
		const user = userEvent.setup();
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ lineNumber: 42 })]}
				{...defaultProps}
			/>,
		);

		await user.click(screen.getByText("Fix this"));
		expect(defaultProps.onCommentClick).toHaveBeenCalledWith("src/main.ts", 42);
	});

	it("calls onCommentClick with filePath only for file comments", async () => {
		const user = userEvent.setup();
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ lineNumber: undefined })]}
				{...defaultProps}
			/>,
		);

		await user.click(screen.getByText("Fix this"));
		expect(defaultProps.onCommentClick).toHaveBeenCalledWith(
			"src/main.ts",
			undefined,
		);
	});

	it("calls onDelete when delete button clicked", async () => {
		const user = userEvent.setup();
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment()]}
				{...defaultProps}
				unsentCount={1}
			/>,
		);

		await user.click(screen.getByLabelText("Delete comment"));
		expect(defaultProps.onDelete).toHaveBeenCalledWith("c1");
	});

	it("disables send all button when unsentCount is 0", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ status: "sent" })]}
				{...defaultProps}
				unsentCount={0}
			/>,
		);

		const sendAllButton = screen.getByLabelText("Send all unsent comments");
		expect(sendAllButton).toBeDisabled();
	});

	it("calls onSendAll when send all button clicked", async () => {
		const user = userEvent.setup();
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment()]}
				{...defaultProps}
				unsentCount={1}
			/>,
		);

		await user.click(screen.getByLabelText("Send all unsent comments"));
		expect(defaultProps.onSendAll).toHaveBeenCalled();
	});
});
