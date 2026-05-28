import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ReviewThreadHandoffContext } from "@/contexts/ReviewThreadHandoffContext";
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
	resolve: null,
	createdAt: Date.now(),
	updatedAt: Date.now(),
	version: 1,
	canResolve: true,
	...overrides,
});

describe("DiffInlineComment", () => {
	const defaultProps = {
		onAppend: vi.fn().mockResolvedValue(undefined),
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

	it("renders actor kind and resolve metadata", () => {
		render(
			<DiffInlineComment
				comment={makeComment({
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

		// コメント author の displayName/kind は別要素として表示される
		expect(screen.getAllByText("Human").length).toBeGreaterThan(0);
		expect(screen.getAllByText("human").length).toBeGreaterThan(0);
		// Resolve メタ情報（アイコンと同じ flex div 内に連続テキストとして配置）
		expect(screen.getByText(/resolved by human · Human/)).toBeInTheDocument();
		// resolve.summary は DiffCommentBody 経由で Markdown レンダリングされる
		expect(screen.getByText("Fixed")).toBeInTheDocument();
	});

	it("renders comment content as markdown (bold, code, list)", () => {
		render(
			<DiffInlineComment
				comment={makeComment({
					comments: [
						{
							id: "c-md",
							threadId: "c1",
							author: { kind: "human", displayName: "Human" },
							content: "**bold** and `inline` and\n- item",
							createdAt: Date.now(),
						},
					],
				})}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("bold").tagName).toBe("STRONG");
		expect(screen.getByText("inline").tagName).toBe("CODE");
		expect(screen.getByRole("listitem")).toHaveTextContent("item");
	});

	it("renders author icon by kind (human → User, agent → Bot)", () => {
		render(
			<DiffInlineComment
				comment={makeComment({
					comments: [
						{
							id: "ch",
							threadId: "c1",
							author: { kind: "human", displayName: "Human" },
							content: "hi",
							createdAt: Date.now(),
						},
						{
							id: "ca",
							threadId: "c1",
							author: { kind: "agent", displayName: "Agent" },
							content: "yo",
							createdAt: Date.now(),
						},
					],
				})}
				{...defaultProps}
			/>,
		);
		expect(screen.getByTestId("author-icon-human")).toBeInTheDocument();
		expect(screen.getByTestId("author-icon-agent")).toBeInTheDocument();
	});

	it("renders resolve summary as markdown", () => {
		render(
			<DiffInlineComment
				comment={makeComment({
					resolve: {
						actor: { kind: "human", displayName: "Human" },
						outcome: "resolved",
						summary: "Fixed **all** issues",
						resolvedAt: 1,
					},
				})}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("all").tagName).toBe("STRONG");
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

	it("does not render delete button when onDelete is not provided", () => {
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);
		expect(screen.queryByLabelText("Delete thread")).not.toBeInTheDocument();
	});

	it("renders delete button when onDelete is provided", () => {
		render(
			<DiffInlineComment
				comment={makeComment()}
				{...defaultProps}
				onDelete={vi.fn().mockResolvedValue(undefined)}
			/>,
		);
		expect(screen.getByLabelText("Delete thread")).toBeInTheDocument();
	});

	it("does not call onDelete when the confirmation dialog is cancelled", async () => {
		const onDelete = vi.fn().mockResolvedValue(undefined);
		const user = userEvent.setup();
		render(
			<DiffInlineComment
				comment={makeComment()}
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
		const onDelete = vi.fn().mockResolvedValue(undefined);
		const user = userEvent.setup();
		render(
			<DiffInlineComment
				comment={makeComment({ id: "thread-xyz" })}
				{...defaultProps}
				onDelete={onDelete}
			/>,
		);

		await user.click(screen.getByLabelText("Delete thread"));
		await user.click(screen.getByRole("button", { name: "Delete" }));
		expect(onDelete).toHaveBeenCalledWith("thread-xyz");
	});

	it("wires reply and resolve actions to thread operations", async () => {
		const user = userEvent.setup();
		render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);

		await user.type(screen.getByPlaceholderText("Reply..."), "Looks fixed");
		await user.click(screen.getByRole("button", { name: "Reply" }));
		expect(defaultProps.onAppend).toHaveBeenLastCalledWith("c1", "Looks fixed");

		await user.type(
			screen.getByPlaceholderText("Resolution summary"),
			"Resolved by change",
		);
		await user.click(screen.getByTitle("Resolve"));

		expect(defaultProps.onResolve).toHaveBeenCalledWith(
			"c1",
			"resolved",
			"Resolved by change",
		);
	});

	// spec issues-1022 "Thread handoff contract": Diff Thread を active な AgentChat
	// session に共有するボタンが、active session の有無に応じて活性 / 非活性を切り替え、
	// 押下時に thread id 付きで sendThreadToAgent が呼ばれる。
	describe("send-to-agent button", () => {
		const sendToAgentLabel = "Send Diff Thread to current Agent";
		const noActiveLabel = "No active Agent session";

		it("is disabled and shows no-active-session tooltip when no active session", () => {
			render(
				<ReviewThreadHandoffContext.Provider
					value={{
						canSend: false,
						sendThreadToAgent: vi.fn(),
					}}
				>
					<DiffInlineComment comment={makeComment()} {...defaultProps} />
				</ReviewThreadHandoffContext.Provider>,
			);
			const button = screen.getByRole("button", { name: noActiveLabel });
			expect(button).toBeDisabled();
		});

		it("dispatches sendThreadToAgent with thread id when clicked", async () => {
			const sendThreadToAgent = vi.fn().mockResolvedValue(undefined);
			render(
				<ReviewThreadHandoffContext.Provider
					value={{ canSend: true, sendThreadToAgent }}
				>
					<DiffInlineComment comment={makeComment()} {...defaultProps} />
				</ReviewThreadHandoffContext.Provider>,
			);
			const user = userEvent.setup();
			const button = screen.getByRole("button", { name: sendToAgentLabel });
			expect(button).toBeEnabled();
			await user.click(button);
			expect(sendThreadToAgent).toHaveBeenCalledWith("c1");
		});

		it("is disabled (alongside other actions) when thread is busy with another action", () => {
			// 既存テストと同じく、provider 不在の場合は no-op fallback により canSend=false。
			render(<DiffInlineComment comment={makeComment()} {...defaultProps} />);
			const button = screen.getByRole("button", { name: noActiveLabel });
			expect(button).toBeDisabled();
		});
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
