import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { WorkflowDocumentViewer } from "./WorkflowDocumentViewer";

describe("WorkflowDocumentViewer", () => {
	it("renders markdown content", () => {
		render(<WorkflowDocumentViewer content="# Hello World" />);

		expect(screen.getByTestId("workflow-document-viewer")).toBeInTheDocument();
		expect(screen.getByText("Hello World")).toBeInTheDocument();
	});

	it("renders with custom className", () => {
		render(<WorkflowDocumentViewer content="test" className="custom-class" />);

		const el = screen.getByTestId("workflow-document-viewer");
		expect(el).toHaveClass("custom-class");
	});

	it("renders empty content without error", () => {
		render(<WorkflowDocumentViewer content="" />);

		expect(screen.getByTestId("workflow-document-viewer")).toBeInTheDocument();
	});

	it("calls onCreateThread with line number when comment button is clicked", async () => {
		const user = userEvent.setup();
		const onCreateThread = vi.fn();

		render(
			<WorkflowDocumentViewer
				content={"# Heading\n\nParagraph"}
				onCreateThread={onCreateThread}
			/>,
		);

		const buttons = screen.getAllByRole("button", {
			name: /Comment on line/,
		});
		expect(buttons.length).toBeGreaterThan(0);

		await user.click(buttons[0]);
		expect(onCreateThread).toHaveBeenCalledOnce();
		expect(onCreateThread).toHaveBeenCalledWith(expect.any(Number));
	});

	it("does not render comment buttons when onCreateThread is not provided", () => {
		render(<WorkflowDocumentViewer content={"# Heading\n\nParagraph"} />);

		const buttons = screen.queryAllByRole("button", {
			name: /Comment on line/,
		});
		expect(buttons).toHaveLength(0);
	});
});
