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

	describe("split mode", () => {
		it("renders grid container with separator", () => {
			render(
				<MarkdownDiffViewer
					originalContent="original"
					modifiedContent="modified"
					diffMode="split"
				/>,
			);
			expect(screen.getByTestId("md-split-grid")).toBeInTheDocument();
			const grid = screen.getByTestId("md-split-grid");
			const separators = grid.querySelectorAll(".md-split-separator");
			expect(separators.length).toBeGreaterThan(0);
		});

		it("renders both original and modified content", () => {
			render(
				<MarkdownDiffViewer
					originalContent="original text"
					modifiedContent="modified text"
					diffMode="split"
				/>,
			);
			expect(screen.getByText("original text")).toBeInTheDocument();
			expect(screen.getByText("modified text")).toBeInTheDocument();
		});

		it("applies deleted/added cell classes for modified content", () => {
			render(
				<MarkdownDiffViewer
					originalContent="old text"
					modifiedContent="new text"
					diffMode="split"
				/>,
			);
			const grid = screen.getByTestId("md-split-grid");
			expect(grid.querySelector(".md-split-cell-deleted")).toBeInTheDocument();
			expect(grid.querySelector(".md-split-cell-added")).toBeInTheDocument();
		});
	});

	describe("inline mode", () => {
		it("renders added chunks with inline-added class", () => {
			render(
				<MarkdownDiffViewer
					originalContent=""
					modifiedContent="new line"
					diffMode="inline"
				/>,
			);
			const viewer = screen.getByTestId("markdown-diff-viewer");
			const addedDiv = viewer.querySelector(".md-diff-inline-added");
			expect(addedDiv).toBeInTheDocument();
		});

		it("renders removed chunks with inline-removed class", () => {
			render(
				<MarkdownDiffViewer
					originalContent="old line"
					modifiedContent=""
					diffMode="inline"
				/>,
			);
			const viewer = screen.getByTestId("markdown-diff-viewer");
			const removedDiv = viewer.querySelector(".md-diff-inline-removed");
			expect(removedDiv).toBeInTheDocument();
		});

		it("renders unchanged chunks without diff class", () => {
			render(
				<MarkdownDiffViewer
					originalContent="unchanged\nold line\n"
					modifiedContent="unchanged\nnew line\n"
					diffMode="inline"
				/>,
			);
			const viewer = screen.getByTestId("markdown-diff-viewer");
			const firstChild = viewer.children[0];
			expect(firstChild.classList.contains("md-diff-inline-added")).toBe(false);
			expect(firstChild.classList.contains("md-diff-inline-removed")).toBe(
				false,
			);
		});
	});
});
