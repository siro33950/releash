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
	stances: [],
	resolve: null,
	createdAt: Date.now(),
	updatedAt: Date.now(),
	version: 1,
	canResolve: true,
	myStance: "none",
	...overrides,
});

describe("DiffCommentList", () => {
	const defaultProps = {
		onCommentClick: vi.fn(),
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

	it("calls onCommentClick with filePath and lineNumber", async () => {
		const user = userEvent.setup();
		renderWithProviders(
			<DiffCommentList
				comments={[
					makeComment({
						target: { filePath: "src/main.ts", lineNumber: 42, endLine: null },
					}),
				]}
				{...defaultProps}
			/>,
		);

		await user.click(screen.getByText("Fix this"));
		expect(defaultProps.onCommentClick).toHaveBeenCalledWith("src/main.ts", 42);
	});

	it("shows and selects general threads without a file position", async () => {
		const user = userEvent.setup();
		renderWithProviders(
			<DiffCommentList
				comments={[
					makeComment({
						target: { filePath: null, lineNumber: null, endLine: null },
						comments: [
							{
								id: "c1",
								threadId: "t1",
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
		expect(defaultProps.onCommentClick).toHaveBeenCalledWith("", undefined);
	});
});
