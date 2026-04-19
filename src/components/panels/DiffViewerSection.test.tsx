import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DiffViewerSection } from "./DiffViewerSection";

vi.mock("./ImageDiffViewer", () => ({
	ImageDiffViewer: () => <div data-testid="image-diff-viewer" />,
}));

vi.mock("./MarkdownDiffViewer", () => ({
	MarkdownDiffViewer: () => <div data-testid="markdown-diff-viewer" />,
}));

vi.mock("./CodeDiffViewer", () => ({
	CodeDiffViewer: () => <div data-testid="code-diff-viewer" />,
}));

const baseProps = {
	isImage: false,
	isMarkdown: false,
	showPreview: false,
	imageDiff: { originalUrl: null, modifiedUrl: null, loading: false },
	originalContent: "original",
	modifiedContent: "modified",
	diffMode: "inline" as const,
};

describe("DiffViewerSection", () => {
	it("renders ImageDiffViewer when isImage is true", () => {
		render(<DiffViewerSection {...baseProps} isImage={true} />);
		expect(screen.getByTestId("image-diff-viewer")).toBeDefined();
	});

	it("renders MarkdownDiffViewer when isMarkdown and showPreview are true", () => {
		render(
			<DiffViewerSection {...baseProps} isMarkdown={true} showPreview={true} />,
		);
		expect(screen.getByTestId("markdown-diff-viewer")).toBeDefined();
	});

	it("renders CodeDiffViewer for non-image non-markdown files", () => {
		render(<DiffViewerSection {...baseProps} />);
		expect(screen.getByTestId("code-diff-viewer")).toBeDefined();
	});

	it("renders CodeDiffViewer when isMarkdown is true but showPreview is false", () => {
		render(
			<DiffViewerSection
				{...baseProps}
				isMarkdown={true}
				showPreview={false}
			/>,
		);
		expect(screen.getByTestId("code-diff-viewer")).toBeDefined();
	});
});
