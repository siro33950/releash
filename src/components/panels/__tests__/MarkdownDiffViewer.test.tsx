import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MarkdownDiffViewer } from "../MarkdownDiffViewer";

describe("MarkdownDiffViewer", () => {
	it("renders modified content as markdown", () => {
		render(
			<MarkdownDiffViewer
				originalContent="hello"
				modifiedContent="**bold text**"
			/>,
		);
		const strong = screen.getByText("bold text");
		expect(strong.tagName).toBe("STRONG");
	});

	it("has data-testid", () => {
		render(<MarkdownDiffViewer originalContent="" modifiedContent="hello" />);
		expect(screen.getByTestId("markdown-diff-viewer")).toBeInTheDocument();
	});

	it("applies diff gutter class to added content", () => {
		render(
			<MarkdownDiffViewer originalContent="" modifiedContent="new line" />,
		);
		const viewer = screen.getByTestId("markdown-diff-viewer");
		const p = viewer.querySelector("p");
		expect(p?.classList.contains("md-diff-gutter-added")).toBe(true);
	});

	it("applies diff gutter class to modified content", () => {
		render(
			<MarkdownDiffViewer
				originalContent="old text"
				modifiedContent="new text"
			/>,
		);
		const viewer = screen.getByTestId("markdown-diff-viewer");
		const p = viewer.querySelector("p");
		expect(p?.classList.contains("md-diff-gutter-modified")).toBe(true);
	});

	it("does not apply diff class when content is identical", () => {
		render(
			<MarkdownDiffViewer
				originalContent="same text"
				modifiedContent="same text"
			/>,
		);
		const viewer = screen.getByTestId("markdown-diff-viewer");
		const p = viewer.querySelector("p");
		expect(p?.classList.contains("md-diff-gutter-added")).toBe(false);
		expect(p?.classList.contains("md-diff-gutter-modified")).toBe(false);
	});
});
