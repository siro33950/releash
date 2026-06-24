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
	hunks: [],
	changeGroups: [],
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

	it("renders fallback notice when review file view is limited", () => {
		render(
			<DiffViewerSection
				{...baseProps}
				fallbackView={{
					kind: "fallback",
					version: 3,
					stale: false,
					fileId: "large.txt",
					path: "large.txt",
					reason: "fileSize",
					totalLines: null,
					sizeBytes: 1_048_577,
					hunkCount: null,
					limited: true,
				}}
			/>,
		);

		expect(screen.getByText("File is too large to preview")).toBeDefined();
		expect(screen.queryByTestId("code-diff-viewer")).toBeNull();
	});

	it("renders an error notice instead of loading when review file view fails", () => {
		render(
			<DiffViewerSection
				{...baseProps}
				hunks={null}
				error="Failed to load diff: review target is not in snapshot"
			/>,
		);

		expect(
			screen.getByText("Failed to load diff: review target is not in snapshot"),
		).toBeDefined();
		expect(screen.queryByText("Loading diff")).toBeNull();
		expect(screen.queryByTestId("code-diff-viewer")).toBeNull();
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

	it("does not render CodeDiffViewer before Rust hunks are available", () => {
		render(<DiffViewerSection {...baseProps} hunks={null} />);
		expect(screen.queryByTestId("code-diff-viewer")).toBeNull();
		expect(screen.getByText("Loading diff")).toBeDefined();
	});
});
