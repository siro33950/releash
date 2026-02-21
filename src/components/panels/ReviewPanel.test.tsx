import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { LineComment } from "@/types/comment";
import { ReviewPanel } from "./ReviewPanel";

vi.mock("./TerminalPanel", () => ({
	TerminalPanel: () => <div data-testid="terminal-panel" />,
}));

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

describe("ReviewPanel", () => {
	it("should render Terminal and Comments tab buttons", () => {
		render(<ReviewPanel comments={[]} />);
		expect(screen.getByText("Terminal")).toBeInTheDocument();
		expect(screen.getByText("Comments")).toBeInTheDocument();
	});

	it("should default to terminal tab", () => {
		render(<ReviewPanel comments={[]} />);
		expect(screen.getByRole("tab", { name: /Terminal/ })).toHaveAttribute(
			"aria-selected",
			"true",
		);
		expect(screen.getByTestId("terminal-panel")).toBeInTheDocument();
	});

	it("should show empty state when switching to comments tab with no comments", async () => {
		const user = userEvent.setup();
		render(<ReviewPanel comments={[]} />);
		await user.click(screen.getByText("Comments"));
		expect(screen.getByText("No comments")).toBeInTheDocument();
	});

	it("should show unsent count badge on comments tab", () => {
		render(
			<ReviewPanel
				comments={[
					makeComment({ id: "c-1", status: "unsent" }),
					makeComment({ id: "c-2", status: "sent" }),
				]}
			/>,
		);
		expect(screen.getByText("1")).toBeInTheDocument();
	});

	it("should show Send button when comments tab is active and unsent comments exist", async () => {
		const user = userEvent.setup();
		render(
			<ReviewPanel comments={[makeComment()]} onSendToTerminal={vi.fn()} />,
		);
		await user.click(screen.getByText("Comments"));
		expect(screen.getByText("Send")).toBeInTheDocument();
	});

	it("should call onSendToTerminal with unsent comments", async () => {
		const user = userEvent.setup();
		const onSend = vi.fn();
		const unsent = makeComment({ id: "c-1", status: "unsent" });
		render(
			<ReviewPanel
				comments={[unsent, makeComment({ id: "c-2", status: "sent" })]}
				onSendToTerminal={onSend}
			/>,
		);
		await user.click(screen.getByText("Comments"));
		await user.click(screen.getByText("Send"));
		expect(onSend).toHaveBeenCalledWith([unsent]);
	});

	it("should pass showSentComments and onToggleShowSent to CommentList", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(
			<ReviewPanel
				comments={[makeComment({ id: "c-1", status: "sent" })]}
				showSentComments={false}
				onToggleShowSent={onToggle}
			/>,
		);
		await user.click(screen.getByText("Comments"));
		const toggleBtn = screen.getByTestId("toggle-sent-comments");
		expect(toggleBtn).toBeInTheDocument();
		await user.click(toggleBtn);
		expect(onToggle).toHaveBeenCalledTimes(1);
	});
});
