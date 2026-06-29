import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { ReviewDiscussionThread } from "@/types/diffComment";
import { DiffCommentList } from "./DiffCommentList";

function renderWithProviders(ui: React.ReactElement) {
	return render(<TooltipProvider>{ui}</TooltipProvider>);
}

const makeComment = (
	overrides: Partial<ReviewDiscussionThread> = {},
): ReviewDiscussionThread => ({
	id: "t1",
	worktreeName: "wt",
	author: { kind: "human", displayName: "Human" },
	target: { filePath: "src/main.ts", lineNumber: 10, endLine: null },
	state: "open",
	comments: [
		{
			id: "c1",
			threadId: "t1",
			author: { kind: "human", displayName: "Human" },
			content: "Fix this",
			createdAt: Date.now(),
		},
	],
	resolve: null,
	createdAt: Date.now(),
	updatedAt: Date.now(),
	version: 1,
	canResolve: true,
	...overrides,
});

describe("DiffCommentList", () => {
	const defaultProps = {
		onThreadClick: vi.fn(),
	};

	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("shows empty state when no threads", () => {
		renderWithProviders(<DiffCommentList comments={[]} {...defaultProps} />);
		expect(screen.getByText("No threads yet")).toBeInTheDocument();
	});

	it("renders header with Threads label", () => {
		renderWithProviders(
			<DiffCommentList comments={[makeComment()]} {...defaultProps} />,
		);
		expect(screen.getByText("Threads")).toBeInTheDocument();
	});

	it("groups threads by file", () => {
		const comments = [
			makeComment({
				id: "t1",
				target: { filePath: "src/a.ts", lineNumber: 10, endLine: null },
				comments: [
					{
						id: "c1",
						threadId: "t1",
						author: { kind: "human", displayName: "Human" },
						content: "Comment A",
						createdAt: Date.now(),
					},
				],
			}),
			makeComment({
				id: "t2",
				target: { filePath: "src/b.ts", lineNumber: 10, endLine: null },
				comments: [
					{
						id: "c2",
						threadId: "t2",
						author: { kind: "human", displayName: "Human" },
						content: "Comment B",
						createdAt: Date.now(),
					},
				],
			}),
		];
		renderWithProviders(
			<DiffCommentList comments={comments} {...defaultProps} />,
		);
		expect(screen.getByText("a.ts")).toBeInTheDocument();
		expect(screen.getByText("b.ts")).toBeInTheDocument();
		expect(screen.getByText("Comment A")).toBeInTheDocument();
		expect(screen.getByText("Comment B")).toBeInTheDocument();
	});

	it("shows line, range, file, and resolved labels", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[
					makeComment({
						id: "t1",
						target: { filePath: "src/a.ts", lineNumber: 42, endLine: null },
					}),
					makeComment({
						id: "t2",
						target: { filePath: "src/a.ts", lineNumber: 10, endLine: 20 },
					}),
					makeComment({
						id: "t3",
						target: { filePath: "src/a.ts", lineNumber: null, endLine: null },
						state: "resolved",
					}),
				]}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("L42")).toBeInTheDocument();
		expect(screen.getByText("L10-20")).toBeInTheDocument();
		expect(screen.getByText("file")).toBeInTheDocument();
		expect(screen.getByText("resolved")).toBeInTheDocument();
	});

	it("renders the initial comment as preview", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[
					makeComment({
						id: "t1",
						comments: [
							{
								id: "c1",
								threadId: "t1",
								author: { kind: "human", displayName: "Human" },
								content: "Initial claim",
								createdAt: 1,
							},
							{
								id: "c2",
								threadId: "t1",
								author: { kind: "human", displayName: "Human" },
								content: "Later reply",
								createdAt: 2,
							},
						],
					}),
				]}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("Initial claim")).toBeInTheDocument();
		expect(screen.queryByText("Later reply")).not.toBeInTheDocument();
	});

	it("calls onThreadClick with target derived from a line thread", async () => {
		const user = userEvent.setup();
		renderWithProviders(
			<DiffCommentList
				comments={[
					makeComment({
						id: "thread-line",
						target: { filePath: "src/main.ts", lineNumber: 42, endLine: null },
					}),
				]}
				{...defaultProps}
			/>,
		);

		await user.click(screen.getByText("Fix this"));
		expect(defaultProps.onThreadClick).toHaveBeenCalledWith({
			filePath: "src/main.ts",
			threadId: "thread-line",
			lineNumber: 42,
			isFileComment: false,
		});
	});

	it("calls onThreadClick with isFileComment=true for file-level threads", async () => {
		const user = userEvent.setup();
		renderWithProviders(
			<DiffCommentList
				comments={[
					makeComment({
						id: "thread-file",
						target: {
							filePath: "src/main.ts",
							lineNumber: null,
							endLine: null,
						},
						comments: [
							{
								id: "c1",
								threadId: "thread-file",
								author: { kind: "human", displayName: "Human" },
								content: "File scope comment",
								createdAt: Date.now(),
							},
						],
					}),
				]}
				{...defaultProps}
			/>,
		);

		await user.click(screen.getByText("File scope comment"));
		expect(defaultProps.onThreadClick).toHaveBeenCalledWith({
			filePath: "src/main.ts",
			threadId: "thread-file",
			lineNumber: undefined,
			isFileComment: true,
		});
	});

	it("shows and selects general threads without a file position", async () => {
		const user = userEvent.setup();
		renderWithProviders(
			<DiffCommentList
				comments={[
					makeComment({
						id: "thread-general",
						target: { filePath: null, lineNumber: null, endLine: null },
						comments: [
							{
								id: "c1",
								threadId: "thread-general",
								author: { kind: "human", displayName: "Human" },
								content: "General discussion",
								createdAt: Date.now(),
							},
						],
					}),
				]}
				{...defaultProps}
			/>,
		);

		expect(screen.getByText("General")).toBeInTheDocument();
		expect(screen.getByText("file")).toBeInTheDocument();
		await user.click(screen.getByText("General discussion"));
		expect(defaultProps.onThreadClick).toHaveBeenCalledWith({
			filePath: "",
			threadId: "thread-general",
			lineNumber: undefined,
			isFileComment: true,
		});
	});

	it("does not render delete button when onDelete is not provided", () => {
		renderWithProviders(
			<DiffCommentList comments={[makeComment()]} {...defaultProps} />,
		);
		expect(screen.queryByLabelText("Delete thread")).not.toBeInTheDocument();
	});

	it("renders a delete button per thread when onDelete is provided", () => {
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ id: "t1" }), makeComment({ id: "t2" })]}
				{...defaultProps}
				onDelete={vi.fn().mockResolvedValue(undefined)}
			/>,
		);
		expect(screen.getAllByLabelText("Delete thread")).toHaveLength(2);
	});

	it("does not fire onThreadClick when the delete button is clicked", async () => {
		const user = userEvent.setup();
		const onDelete = vi.fn().mockResolvedValue(undefined);
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment()]}
				{...defaultProps}
				onDelete={onDelete}
			/>,
		);

		await user.click(screen.getByLabelText("Delete thread"));
		expect(defaultProps.onThreadClick).not.toHaveBeenCalled();
	});

	it("does not call onDelete when the confirmation dialog is cancelled", async () => {
		const user = userEvent.setup();
		const onDelete = vi.fn().mockResolvedValue(undefined);
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment()]}
				{...defaultProps}
				onDelete={onDelete}
			/>,
		);

		await user.click(screen.getByLabelText("Delete thread"));
		expect(
			screen.getByRole("alertdialog", { name: "Delete this thread?" }),
		).toBeInTheDocument();
		await user.click(screen.getByRole("button", { name: "Cancel" }));
		expect(onDelete).not.toHaveBeenCalled();
	});

	it("calls onDelete with the thread id when the confirmation is accepted", async () => {
		const user = userEvent.setup();
		const onDelete = vi.fn().mockResolvedValue(undefined);
		renderWithProviders(
			<DiffCommentList
				comments={[makeComment({ id: "thread-xyz" })]}
				{...defaultProps}
				onDelete={onDelete}
			/>,
		);

		await user.click(screen.getByLabelText("Delete thread"));
		await user.click(screen.getByRole("button", { name: "Delete" }));
		expect(onDelete).toHaveBeenCalledWith("thread-xyz");
	});

	// spec issues-1022 "Thread handoff contract": スレッドパネル各行から、対象 Thread を
	// 現在 active な AgentChat session に共有できる。
	describe("send-to-agent button", () => {
		const noActiveLabel = "No active Agent session";

		it("is disabled when no active AgentChat session", () => {
			renderWithProviders(
				<DiffCommentList
					comments={[makeComment({ id: "thread-xyz" })]}
					{...defaultProps}
				/>,
			);
			expect(
				screen.getByRole("button", { name: noActiveLabel }),
			).toBeDisabled();
		});
	});
});
