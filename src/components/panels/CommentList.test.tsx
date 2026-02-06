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
		...overrides,
	};
}

describe("CommentList", () => {
	it("should show empty state with hints when no comments", () => {
		render(<CommentList comments={[]} />);
		expect(screen.getByText("コメントなし")).toBeInTheDocument();
		expect(screen.getByText(/行番号の左マージンをクリック/)).toBeInTheDocument();
		expect(screen.getByText("⌘K")).toBeInTheDocument();
	});

	it("should display file name and comment content", () => {
		render(
			<CommentList
				comments={[makeComment({ content: "fix this bug" })]}
			/>,
		);
		expect(screen.getByText("App.tsx")).toBeInTheDocument();
		expect(screen.getByText("fix this bug")).toBeInTheDocument();
	});

	it("should display line number", () => {
		render(
			<CommentList
				comments={[makeComment({ lineNumber: 42 })]}
			/>,
		);
		expect(screen.getByText("L42")).toBeInTheDocument();
	});

	it("should show status badge", () => {
		render(
			<CommentList
				comments={[
					makeComment({ id: "c-1", status: "unsent" }),
					makeComment({ id: "c-2", status: "sent", lineNumber: 20 }),
				]}
			/>,
		);
		expect(screen.getByText("unsent")).toBeInTheDocument();
		expect(screen.getByText("sent")).toBeInTheDocument();
	});

	it("should display range line number with endLine", () => {
		render(
			<CommentList
				comments={[makeComment({ lineNumber: 5, endLine: 12 })]}
			/>,
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
});
