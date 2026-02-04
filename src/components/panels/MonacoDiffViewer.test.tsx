import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MonacoDiffViewer } from "./MonacoDiffViewer";

describe("MonacoDiffViewer", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should render toolbar with base selector", () => {
		render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
			/>,
		);

		expect(screen.getByText("Base:")).toBeInTheDocument();
		expect(screen.getByRole("combobox")).toBeInTheDocument();
	});

	it("should render mode toggle buttons", () => {
		render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
			/>,
		);

		expect(screen.getByText("Gutter")).toBeInTheDocument();
		expect(screen.getByText("Inline")).toBeInTheDocument();
		expect(screen.getByText("Split")).toBeInTheDocument();
	});

	it("should have split mode selected by default", () => {
		render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
			/>,
		);

		const splitButton = screen.getByText("Split").closest("button");
		expect(splitButton).toHaveClass("bg-background");
	});

	it("should switch to gutter mode when clicked", () => {
		render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
			/>,
		);

		const gutterButton = screen.getByText("Gutter").closest("button");
		if (gutterButton) {
			fireEvent.click(gutterButton);
		}

		expect(gutterButton).toHaveClass("bg-background");
	});

	it("should switch to inline mode when clicked", () => {
		render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
			/>,
		);

		const inlineButton = screen.getByText("Inline").closest("button");
		if (inlineButton) {
			fireEvent.click(inlineButton);
		}

		expect(inlineButton).toHaveClass("bg-background");
	});

	it("should render base options in select", () => {
		render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
			/>,
		);

		const select = screen.getByRole("combobox");
		const options = select.querySelectorAll("option");

		expect(options).toHaveLength(4);
		expect(options[0]).toHaveTextContent("HEAD");
		expect(options[1]).toHaveTextContent("HEAD~1");
		expect(options[2]).toHaveTextContent("HEAD~5");
		expect(options[3]).toHaveTextContent("Staged");
	});

	it("should call onDiffBaseChange when base is changed", () => {
		const onDiffBaseChange = vi.fn();

		render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
				diffBase="HEAD"
				onDiffBaseChange={onDiffBaseChange}
			/>,
		);

		const select = screen.getByRole("combobox");
		fireEvent.change(select, { target: { value: "HEAD~1" } });

		expect(onDiffBaseChange).toHaveBeenCalledWith("HEAD~1");
	});

	it("should apply custom className", () => {
		const { container } = render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
				className="custom-class"
			/>,
		);

		expect(container.firstChild).toHaveClass("custom-class");
	});

	it("should accept language prop", () => {
		render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
				language="javascript"
			/>,
		);

		expect(screen.getByText("Base:")).toBeInTheDocument();
	});
});
