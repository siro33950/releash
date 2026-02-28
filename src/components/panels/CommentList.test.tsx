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
				showSentComments={true}
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

	it("should hide sent comments when showSentComments is false", () => {
		render(
			<CommentList
				comments={[
					makeComment({ id: "c-1", status: "unsent", content: "unsent one" }),
					makeComment({
						id: "c-2",
						status: "sent",
						lineNumber: 20,
						content: "sent one",
					}),
				]}
				showSentComments={false}
				onToggleShowSent={vi.fn()}
			/>,
		);
		expect(screen.getByText("unsent one")).toBeInTheDocument();
		expect(screen.queryByText("sent one")).not.toBeInTheDocument();
	});

	it("should show sent comments when showSentComments is true", () => {
		render(
			<CommentList
				comments={[
					makeComment({ id: "c-1", status: "unsent", content: "unsent one" }),
					makeComment({
						id: "c-2",
						status: "sent",
						lineNumber: 20,
						content: "sent one",
					}),
				]}
				showSentComments={true}
				onToggleShowSent={vi.fn()}
			/>,
		);
		expect(screen.getByText("unsent one")).toBeInTheDocument();
		expect(screen.getByText("sent one")).toBeInTheDocument();
	});

	it("should show toggle button with sent count", () => {
		render(
			<CommentList
				comments={[
					makeComment({ id: "c-1", status: "sent" }),
					makeComment({ id: "c-2", status: "sent", lineNumber: 20 }),
				]}
				showSentComments={false}
				onToggleShowSent={vi.fn()}
			/>,
		);
		expect(screen.getByTestId("toggle-sent-comments")).toBeInTheDocument();
		expect(screen.getByText(/Sent \(2\)/)).toBeInTheDocument();
	});

	it("should call onToggleShowSent when toggle button is clicked", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(
			<CommentList
				comments={[makeComment({ id: "c-1", status: "sent" })]}
				showSentComments={false}
				onToggleShowSent={onToggle}
			/>,
		);
		await user.click(screen.getByTestId("toggle-sent-comments"));
		expect(onToggle).toHaveBeenCalledTimes(1);
	});

	it("should not show toggle button when no sent comments", () => {
		render(
			<CommentList
				comments={[makeComment({ id: "c-1", status: "unsent" })]}
				showSentComments={false}
				onToggleShowSent={vi.fn()}
			/>,
		);
		expect(
			screen.queryByTestId("toggle-sent-comments"),
		).not.toBeInTheDocument();
	});
});
