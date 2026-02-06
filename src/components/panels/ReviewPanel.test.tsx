import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { LineComment } from "@/types/comment";
import { ReviewPanel } from "./ReviewPanel";

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
	it("should render Comments header", () => {
		render(
			<ReviewPanel
				comments={[]}
			/>,
		);
		expect(screen.getByText("Comments")).toBeInTheDocument();
	});

	it("should show empty state when no comments", () => {
		render(
			<ReviewPanel
				comments={[]}
			/>,
		);
		expect(screen.getByText("コメントなし")).toBeInTheDocument();
	});

	it("should show unsent count badge", () => {
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

	it("should show Send button when unsent comments exist", () => {
		render(
			<ReviewPanel
				comments={[makeComment()]}
				onSendToTerminal={vi.fn()}
			/>,
		);
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
		await user.click(screen.getByText("Send"));
		expect(onSend).toHaveBeenCalledWith([unsent]);
	});
});
