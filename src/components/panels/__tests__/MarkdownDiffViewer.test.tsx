import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DiffRange, InlineChunk, SplitRow } from "@/types/markdown-diff";
import type { DiffMode } from "@/types/settings";
import { MarkdownDiffViewer } from "../MarkdownDiffViewer";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

function deferred<T>() {
	let resolve: (value: T) => void = () => {};
	let reject: (reason?: unknown) => void = () => {};
	const promise = new Promise<T>((nextResolve, nextReject) => {
		resolve = nextResolve;
		reject = nextReject;
	});
	return { promise, resolve, reject };
}

function mockReadModel({
	ranges = [],
	rows = [],
	chunks = [],
	visibleBlocks = [],
	rejectCommands = {},
}: {
	ranges?: DiffRange[];
	rows?: SplitRow[];
	chunks?: InlineChunk[];
	visibleBlocks?: unknown[];
	rejectCommands?: Record<string, unknown>;
} = {}) {
	mockInvoke.mockImplementation((command: string) => {
		if (rejectCommands[command]) {
			return Promise.reject(rejectCommands[command]);
		}

		switch (command) {
			case "compute_markdown_diff_ranges":
				return Promise.resolve(ranges);
			case "compute_markdown_split_rows":
				return Promise.resolve(rows);
			case "compute_markdown_inline_chunks":
				return Promise.resolve(chunks);
			case "compute_visible_markdown_blocks":
				return Promise.resolve(visibleBlocks);
			default:
				return Promise.resolve([]);
		}
	});
}

describe("MarkdownDiffViewer", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		mockReadModel();
	});

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

	it("applies backend diff gutter class to added content", async () => {
		mockReadModel({
			ranges: [{ startLine: 1, endLine: 1, type: "added" }],
		});
		render(
			<MarkdownDiffViewer originalContent="" modifiedContent="new line" />,
		);
		const viewer = screen.getByTestId("markdown-diff-viewer");
		await waitFor(() => {
			const p = viewer.querySelector("p");
			expect(p?.classList.contains("md-diff-gutter-added")).toBe(true);
		});
		expect(mockInvoke).toHaveBeenCalledWith("compute_markdown_diff_ranges", {
			original: "",
			modified: "new line",
			side: "modified",
		});
	});

	it("applies backend diff gutter class to modified content", async () => {
		mockReadModel({
			ranges: [{ startLine: 1, endLine: 1, type: "modified" }],
		});
		render(
			<MarkdownDiffViewer
				originalContent="old text"
				modifiedContent="new text"
			/>,
		);
		const viewer = screen.getByTestId("markdown-diff-viewer");
		await waitFor(() => {
			const p = viewer.querySelector("p");
			expect(p?.classList.contains("md-diff-gutter-modified")).toBe(true);
		});
	});

	it("does not apply diff class when backend returns no ranges", async () => {
		render(
			<MarkdownDiffViewer
				originalContent="same text"
				modifiedContent="same text"
			/>,
		);
		const viewer = screen.getByTestId("markdown-diff-viewer");
		await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
		const p = viewer.querySelector("p");
		expect(p?.classList.contains("md-diff-gutter-added")).toBe(false);
		expect(p?.classList.contains("md-diff-gutter-modified")).toBe(false);
	});

	it.each([
		{
			diffMode: undefined,
			command: "compute_markdown_diff_ranges",
			label: "gutter",
		},
		{
			diffMode: "split" as DiffMode,
			command: "compute_markdown_split_rows",
			label: "split",
		},
		{
			diffMode: "inline" as DiffMode,
			command: "compute_markdown_inline_chunks",
			label: "inline",
		},
	])(
		"shows backend read model errors in $label mode",
		async ({ diffMode, command }) => {
			mockReadModel({ rejectCommands: { [command]: "backend failed" } });
			render(
				<MarkdownDiffViewer
					originalContent="old text"
					modifiedContent="new text"
					diffMode={diffMode}
				/>,
			);

			const alert = await screen.findByRole("alert");
			expect(alert.textContent).toBe("backend failed");
		},
	);

	it("shows coded backend read model message without decoration", async () => {
		mockReadModel({
			rejectCommands: {
				compute_markdown_diff_ranges: {
					code: "MARKDOWN_DIFF_UNAVAILABLE",
					message: "coded backend failed",
				},
			},
		});
		render(
			<MarkdownDiffViewer
				originalContent="old text"
				modifiedContent="new text"
			/>,
		);

		expect((await screen.findByRole("alert")).textContent).toBe(
			"coded backend failed",
		);
	});

	describe("split mode", () => {
		it("renders grid container with separator", async () => {
			mockReadModel({
				rows: [{ left: "original", right: "modified", type: "modified" }],
			});
			render(
				<MarkdownDiffViewer
					originalContent="original"
					modifiedContent="modified"
					diffMode="split"
				/>,
			);
			expect(screen.getByTestId("md-split-grid")).toBeInTheDocument();
			const grid = screen.getByTestId("md-split-grid");
			await waitFor(() => {
				const separators = grid.querySelectorAll(".md-split-separator");
				expect(separators.length).toBeGreaterThan(0);
			});
			expect(mockInvoke).toHaveBeenCalledWith("compute_markdown_split_rows", {
				original: "original",
				modified: "modified",
			});
		});

		it("renders both original and modified content", async () => {
			mockReadModel({
				rows: [
					{ left: "original text", right: "modified text", type: "modified" },
				],
			});
			render(
				<MarkdownDiffViewer
					originalContent="original text"
					modifiedContent="modified text"
					diffMode="split"
				/>,
			);
			expect(await screen.findByText("original text")).toBeInTheDocument();
			expect(await screen.findByText("modified text")).toBeInTheDocument();
		});

		it("applies deleted/added cell classes for backend modified row", async () => {
			mockReadModel({
				rows: [{ left: "old text", right: "new text", type: "modified" }],
			});
			render(
				<MarkdownDiffViewer
					originalContent="old text"
					modifiedContent="new text"
					diffMode="split"
				/>,
			);
			const grid = screen.getByTestId("md-split-grid");
			await waitFor(() => {
				expect(
					grid.querySelector(".md-split-cell-deleted"),
				).toBeInTheDocument();
				expect(grid.querySelector(".md-split-cell-added")).toBeInTheDocument();
			});
		});

		it("renders backend added row with empty left and added right cell", async () => {
			mockReadModel({
				rows: [{ left: null, right: "new text", type: "added" }],
			});
			render(
				<MarkdownDiffViewer
					originalContent=""
					modifiedContent="new text"
					diffMode="split"
				/>,
			);
			const grid = screen.getByTestId("md-split-grid");
			await screen.findByText("new text");

			const cells = grid.querySelectorAll(".md-split-cell");
			expect(cells[0]).toHaveClass("md-split-cell-empty");
			expect(cells[0].textContent).toBe("");
			expect(cells[1]).toHaveClass("md-split-cell-added");
		});
	});

	describe("stale read models", () => {
		it("does not apply previous gutter ranges while the next input is loading", async () => {
			const pendingRanges = deferred<DiffRange[]>();
			mockInvoke.mockImplementation((command: string, args: unknown) => {
				if (command !== "compute_markdown_diff_ranges") {
					return Promise.resolve([]);
				}
				const { modified } = args as { modified: string };
				if (modified === "file A") {
					return Promise.resolve([{ startLine: 1, endLine: 1, type: "added" }]);
				}
				return pendingRanges.promise;
			});

			const { rerender } = render(
				<MarkdownDiffViewer originalContent="" modifiedContent="file A" />,
			);
			const viewer = screen.getByTestId("markdown-diff-viewer");
			await waitFor(() => {
				expect(
					viewer.querySelector(".md-diff-gutter-added"),
				).toBeInTheDocument();
			});

			rerender(
				<MarkdownDiffViewer originalContent="" modifiedContent="file B" />,
			);

			await screen.findByText("file B");
			const paragraph = viewer.querySelector("p");
			expect(paragraph).toHaveTextContent("file B");
			expect(paragraph).not.toHaveClass("md-diff-gutter-added");
		});

		it("does not render previous split rows after the next input fails", async () => {
			const failedRows = deferred<SplitRow[]>();
			mockInvoke.mockImplementation((command: string, args: unknown) => {
				if (command !== "compute_markdown_split_rows") {
					return Promise.resolve([]);
				}
				const { modified } = args as { modified: string };
				if (modified === "file A") {
					return Promise.resolve([
						{ left: "old A", right: "new A", type: "modified" },
					]);
				}
				return failedRows.promise;
			});

			const { rerender } = render(
				<MarkdownDiffViewer
					originalContent="old A"
					modifiedContent="file A"
					diffMode="split"
				/>,
			);
			await screen.findByText("new A");

			rerender(
				<MarkdownDiffViewer
					originalContent="old B"
					modifiedContent="file B"
					diffMode="split"
				/>,
			);
			await act(async () => {
				failedRows.reject(new Error("B failed"));
				await failedRows.promise.catch(() => {});
			});

			expect((await screen.findByRole("alert")).textContent).toBe("B failed");
			expect(screen.queryByText("old A")).not.toBeInTheDocument();
			expect(screen.queryByText("new A")).not.toBeInTheDocument();
			expect(
				screen
					.getByTestId("md-split-grid")
					.querySelector(".md-split-cell-added"),
			).not.toBeInTheDocument();
		});

		it("does not render previous inline chunks while the next input is loading", async () => {
			const pendingChunks = deferred<InlineChunk[]>();
			mockInvoke.mockImplementation((command: string, args: unknown) => {
				if (command !== "compute_markdown_inline_chunks") {
					return Promise.resolve([]);
				}
				const { modified } = args as { modified: string };
				if (modified === "file A") {
					return Promise.resolve([{ content: "new A", type: "added" }]);
				}
				return pendingChunks.promise;
			});

			const { rerender } = render(
				<MarkdownDiffViewer
					originalContent=""
					modifiedContent="file A"
					diffMode="inline"
				/>,
			);
			const viewer = screen.getByTestId("markdown-diff-viewer");
			await screen.findByText("new A");
			expect(viewer.querySelector(".md-diff-inline-added")).toBeInTheDocument();

			rerender(
				<MarkdownDiffViewer
					originalContent=""
					modifiedContent="file B"
					diffMode="inline"
				/>,
			);

			await waitFor(() => {
				expect(screen.queryByText("new A")).not.toBeInTheDocument();
			});
			expect(
				viewer.querySelector(".md-diff-inline-added"),
			).not.toBeInTheDocument();
		});
	});

	describe("inline mode", () => {
		it("renders backend added chunks with inline-added class", async () => {
			mockReadModel({
				chunks: [{ content: "new line", type: "added" }],
			});
			render(
				<MarkdownDiffViewer
					originalContent=""
					modifiedContent="new line"
					diffMode="inline"
				/>,
			);
			const viewer = screen.getByTestId("markdown-diff-viewer");
			await waitFor(() => {
				expect(
					viewer.querySelector(".md-diff-inline-added"),
				).toBeInTheDocument();
			});
			expect(mockInvoke).toHaveBeenCalledWith(
				"compute_markdown_inline_chunks",
				{
					original: "",
					modified: "new line",
				},
			);
		});

		it("renders backend removed chunks with inline-removed class", async () => {
			mockReadModel({
				chunks: [{ content: "old line", type: "removed" }],
			});
			render(
				<MarkdownDiffViewer
					originalContent="old line"
					modifiedContent=""
					diffMode="inline"
				/>,
			);
			const viewer = screen.getByTestId("markdown-diff-viewer");
			await waitFor(() => {
				expect(
					viewer.querySelector(".md-diff-inline-removed"),
				).toBeInTheDocument();
			});
		});

		it("renders backend unchanged chunks without diff class", async () => {
			mockReadModel({
				chunks: [{ content: "same text", type: "unchanged" }],
			});
			render(
				<MarkdownDiffViewer
					originalContent="same text"
					modifiedContent="same text"
					diffMode="inline"
				/>,
			);
			const viewer = screen.getByTestId("markdown-diff-viewer");
			await screen.findByText("same text");
			expect(
				viewer.querySelector(".md-diff-inline-added"),
			).not.toBeInTheDocument();
			expect(
				viewer.querySelector(".md-diff-inline-removed"),
			).not.toBeInTheDocument();
		});
	});

	describe("diff-only mode", () => {
		it("renders backend visible blocks and calls backend read model", async () => {
			mockReadModel({
				visibleBlocks: [
					{
						startLine: 2,
						endLine: 2,
						content: "changed",
					},
				],
			});
			render(
				<MarkdownDiffViewer
					originalContent={"intro\nold\ntail"}
					modifiedContent={"intro\nchanged\ntail"}
					diffOnlyMode={true}
				/>,
			);

			expect(await screen.findByText("changed")).toBeInTheDocument();
			expect(mockInvoke).toHaveBeenCalledWith(
				"compute_visible_markdown_blocks",
				{
					original: "intro\nold\ntail",
					modified: "intro\nchanged\ntail",
					contextLines: 3,
				},
			);
		});

		it("expands hidden gaps from backend visible blocks", async () => {
			mockReadModel({
				visibleBlocks: [
					{
						startLine: 2,
						endLine: 2,
						content: "changed",
					},
				],
			});
			render(
				<MarkdownDiffViewer
					originalContent={"intro\nold\ntail"}
					modifiedContent={"intro\nchanged\ntail"}
					diffOnlyMode={true}
				/>,
			);

			const buttons = await screen.findAllByRole("button", {
				name: /1 lines hidden/,
			});
			fireEvent.click(buttons[0]);
			expect(await screen.findByText("intro")).toBeInTheDocument();
		});

		it("shows backend read model errors instead of no-changes state", async () => {
			mockReadModel({
				rejectCommands: {
					compute_visible_markdown_blocks: "visible blocks failed",
				},
			});
			render(
				<MarkdownDiffViewer
					originalContent="old text"
					modifiedContent="new text"
					diffOnlyMode={true}
				/>,
			);

			const alert = await screen.findByRole("alert");
			expect(alert.textContent).toBe("visible blocks failed");
			expect(screen.queryByText("No changes")).not.toBeInTheDocument();
		});
	});
});
