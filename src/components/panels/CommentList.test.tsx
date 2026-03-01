import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { LineComment } from "@/types/comment";
import { CommentList } from "./CommentList";

function makeComment(overrides: Partial<LineComment> = {}): LineComment {
	return {
		id: "c-1",
		filePath: "/src/App.tsx",
		lineNumber: 10,
		content: "test comment",
		status: "unsent",
		createdAt: Date.now(),
		resolved: false,
		target: "local",
		...overrides,
	};
}

describe("CommentList", () => {
	it("should show empty state with hints when no comments", () => {
		render(<CommentList comments={[]} />);
		expect(screen.getByText("No comments")).toBeInTheDocument();
		expect(
			screen.getByText(/Click the left margin of a line number/),
		).toBeInTheDocument();
		expect(screen.getByText("⌘K")).toBeInTheDocument();
	});

	it("should display file name and comment content", () => {
		render(
			<CommentList comments={[makeComment({ content: "fix this bug" })]} />,
		);
		expect(screen.getByText("App.tsx")).toBeInTheDocument();
		expect(screen.getByText("fix this bug")).toBeInTheDocument();
	});

	it("should display line number", () => {
		render(<CommentList comments={[makeComment({ lineNumber: 42 })]} />);
		expect(screen.getByText("L42")).toBeInTheDocument();
	});

	it("should show status badge", () => {
		render(
			<CommentList
				comments={[
					makeComment({ id: "c-1", status: "unsent" }),
					makeComment({ id: "c-2", status: "sent", lineNumber: 20 }),
				]}
				showResolvedComments={true}
			/>,
		);
		expect(screen.getByText("unsent")).toBeInTheDocument();
		expect(screen.getByText("sent")).toBeInTheDocument();
	});

	it("should display range line number with endLine", () => {
		render(
			<CommentList comments={[makeComment({ lineNumber: 5, endLine: 12 })]} />,
		);
		expect(screen.getByText("L5-12")).toBeInTheDocument();
	});

	it("should call onCommentClick when comment is clicked", async () => {
		const user = userEvent.setup();
		const onClick = vi.fn();
		render(
			<CommentList
				comments={[makeComment({ lineNumber: 15, content: "click me" })]}
				onCommentClick={onClick}
			/>,
		);
		await user.click(screen.getByText("click me"));
		expect(onClick).toHaveBeenCalledWith("/src/App.tsx", 15);
	});

	it("should hide resolved comments when showResolvedComments is false", () => {
		render(
			<CommentList
				comments={[
					makeComment({ id: "c-1", content: "active one" }),
					makeComment({
						id: "c-2",
						resolved: true,
						lineNumber: 20,
						content: "resolved one",
					}),
				]}
				showResolvedComments={false}
				onToggleShowResolved={vi.fn()}
			/>,
		);
		expect(screen.getByText("active one")).toBeInTheDocument();
		expect(screen.queryByText("resolved one")).not.toBeInTheDocument();
	});

	it("should show resolved comments when showResolvedComments is true", () => {
		render(
			<CommentList
				comments={[
					makeComment({ id: "c-1", content: "active one" }),
					makeComment({
						id: "c-2",
						resolved: true,
						lineNumber: 20,
						content: "resolved one",
					}),
				]}
				showResolvedComments={true}
				onToggleShowResolved={vi.fn()}
			/>,
		);
		expect(screen.getByText("active one")).toBeInTheDocument();
		expect(screen.getByText("resolved one")).toBeInTheDocument();
	});

	it("should show toggle button with resolved count", () => {
		render(
			<CommentList
				comments={[
					makeComment({ id: "c-1", resolved: true }),
					makeComment({ id: "c-2", resolved: true, lineNumber: 20 }),
				]}
				showResolvedComments={false}
				onToggleShowResolved={vi.fn()}
			/>,
		);
		expect(screen.getByTestId("toggle-resolved-comments")).toBeInTheDocument();
		expect(screen.getByText(/Resolved \(2\)/)).toBeInTheDocument();
	});

	it("should call onToggleShowResolved when toggle button is clicked", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(
			<CommentList
				comments={[makeComment({ id: "c-1", resolved: true })]}
				showResolvedComments={false}
				onToggleShowResolved={onToggle}
			/>,
		);
		await user.click(screen.getByTestId("toggle-resolved-comments"));
		expect(onToggle).toHaveBeenCalledTimes(1);
	});

	it("should not show toggle button when no resolved comments", () => {
		render(
			<CommentList
				comments={[makeComment({ id: "c-1" })]}
				showResolvedComments={false}
				onToggleShowResolved={vi.fn()}
			/>,
		);
		expect(
			screen.queryByTestId("toggle-resolved-comments"),
		).not.toBeInTheDocument();
	});

	it("should call onDeleteComment when delete button is clicked", async () => {
		const user = userEvent.setup();
		const onDelete = vi.fn();
		render(
			<CommentList
				comments={[makeComment({ id: "c-42", content: "delete me" })]}
				onDeleteComment={onDelete}
			/>,
		);
		const row =
			screen
				.getByRole("button", { name: /delete me/i })
				.closest("[role='button']") ??
			screen.getByText("delete me").closest("[role='button']");
		await user.hover(row!);
		const deleteBtn = screen.getByLabelText("Delete comment");
		await user.click(deleteBtn);
		expect(onDelete).toHaveBeenCalledWith("c-42");
	});

	it("should call onResolveComment when resolve button is clicked", async () => {
		const user = userEvent.setup();
		const onResolve = vi.fn();
		render(
			<CommentList
				comments={[makeComment({ id: "c-99", content: "resolve me" })]}
				onResolveComment={onResolve}
			/>,
		);
		const row = screen.getByText("resolve me").closest("[role='button']");
		await user.hover(row!);
		const resolveBtn = screen.getByLabelText("Resolve comment");
		await user.click(resolveBtn);
		expect(onResolve).toHaveBeenCalledWith("c-99");
	});

	it("should not show resolve button for already resolved comments", async () => {
		const user = userEvent.setup();
		render(
			<CommentList
				comments={[
					makeComment({ id: "c-1", content: "already done", resolved: true }),
				]}
				showResolvedComments={true}
				onResolveComment={vi.fn()}
				onDeleteComment={vi.fn()}
			/>,
		);
		const row = screen.getByText("already done").closest("[role='button']");
		await user.hover(row!);
		expect(screen.queryByLabelText("Resolve comment")).not.toBeInTheDocument();
		expect(screen.getByLabelText("Delete comment")).toBeInTheDocument();
	});

	it("should not show action buttons when callbacks are not provided", () => {
		render(
			<CommentList
				comments={[makeComment({ id: "c-1", content: "no actions" })]}
			/>,
		);
		expect(screen.queryByLabelText("Delete comment")).not.toBeInTheDocument();
		expect(screen.queryByLabelText("Resolve comment")).not.toBeInTheDocument();
	});
});
