import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ShikiDiffViewer } from "./ShikiDiffViewer";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue([]),
}));

vi.mock("@/hooks/useShikiHighlighter", () => ({
	useShikiHighlighter: () => null,
}));

vi.mock("@tanstack/react-virtual", () => ({
	useVirtualizer: ({
		count,
		estimateSize,
	}: {
		count: number;
		estimateSize: (index: number) => number;
	}) => ({
		getVirtualItems: () =>
			Array.from({ length: count }, (_, i) => {
				const size = estimateSize(i);
				return {
					index: i,
					key: i,
					start: i * size,
					size,
					end: (i + 1) * size,
				};
			}),
		getTotalSize: () => {
			let total = 0;
			for (let i = 0; i < count; i++) {
				total += estimateSize(i);
			}
			return total;
		},
		measureElement: () => {},
	}),
}));

const baseProps = {
	originalContent: "line1\nline2\nline3\n",
	modifiedContent: "line1\nmodified\nline3\n",
	language: "typescript",
	hunks: [
		{
			index: 0,
			oldStart: 1,
			oldLines: 3,
			newStart: 1,
			newLines: 3,
			lines: [" line1", "-line2", "+modified", " line3"],
		},
	],
};

describe("ShikiDiffViewer", () => {
	it("renders with data-testid code-diff-viewer", () => {
		render(<ShikiDiffViewer {...baseProps} diffMode="gutter" />);
		expect(screen.getByTestId("code-diff-viewer")).toBeDefined();
	});

	it("renders Gutter mode with new line numbers only", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="gutter" />,
		);
		const diffViewer = container.querySelector(
			"[data-testid='code-diff-viewer']",
		);
		expect(diffViewer).toBeDefined();
		// Gutter mode only shows new line numbers (column count = 1 number column)
		expect(diffViewer?.textContent).toContain("2");
		expect(diffViewer?.textContent).toContain("+");
	});

	it("renders Inline mode showing both old and new line numbers", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="inline" />,
		);
		const diffViewer = container.querySelector(
			"[data-testid='code-diff-viewer']",
		);
		expect(diffViewer).toBeDefined();
		// Inline mode: deleted and added lines are interleaved
		expect(diffViewer?.textContent).toContain("+");
		expect(diffViewer?.textContent).toContain("-");
	});

	it("renders Split mode with two columns", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="split" />,
		);
		const splitColumns = container.querySelectorAll(".flex-1");
		expect(splitColumns.length).toBeGreaterThanOrEqual(2);
	});

	it("displays + marker for added lines and - marker for deleted lines", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="inline" />,
		);
		const text = container.textContent ?? "";
		expect(text).toContain("+");
		expect(text).toContain("-");
	});

	it("shows stage button when changeGroups and onStageGroup are provided", () => {
		const onStage = vi.fn();
		const { container } = render(
			<ShikiDiffViewer
				{...baseProps}
				diffMode="inline"
				changeGroups={[
					{
						groupIndex: 0,
						hunkIndex: 0,
						newStart: 2,
						newEnd: 2,
						lineOffsetStart: 1,
						lineOffsetEnd: 3,
					},
				]}
				onStageGroup={onStage}
				groupActionLabel="Stage"
			/>,
		);
		const stageButtons = container.querySelectorAll(".hunk-stage");
		expect(stageButtons.length).toBeGreaterThan(0);
	});

	it("shows Unstage label when groupActionLabel is Unstage", () => {
		const onStage = vi.fn();
		const { container } = render(
			<ShikiDiffViewer
				{...baseProps}
				diffMode="inline"
				changeGroups={[
					{
						groupIndex: 0,
						hunkIndex: 0,
						newStart: 2,
						newEnd: 2,
						lineOffsetStart: 1,
						lineOffsetEnd: 3,
					},
				]}
				onStageGroup={onStage}
				groupActionLabel="Unstage"
			/>,
		);
		const stageButtons = container.querySelectorAll(".hunk-stage");
		expect(stageButtons.length).toBeGreaterThan(0);
		expect(stageButtons[0].textContent).toBe("Unstage");
	});

	it("renders with plaintext when language is plaintext", () => {
		render(
			<ShikiDiffViewer {...baseProps} diffMode="gutter" language="plaintext" />,
		);
		expect(screen.getByTestId("code-diff-viewer")).toBeDefined();
	});

	it("Gutter mode does not display deleted line content", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="gutter" />,
		);
		const text = container.textContent ?? "";
		expect(text).toContain("modified");
		expect(text).not.toContain("line2");
	});

	it("Gutter mode shows + marker for added lines", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="gutter" />,
		);
		const text = container.textContent ?? "";
		expect(text).toContain("+");
	});

	it("Gutter mode shows - marker for pure deletion", () => {
		const deletionOnlyProps = {
			originalContent: "line1\nremoved\nline2\n",
			modifiedContent: "line1\nline2\n",
			language: "typescript",
			hunks: [
				{
					index: 0,
					oldStart: 1,
					oldLines: 3,
					newStart: 1,
					newLines: 2,
					lines: [" line1", "-removed", " line2"],
				},
			],
		};
		const { container } = render(
			<ShikiDiffViewer {...deletionOnlyProps} diffMode="gutter" />,
		);
		const text = container.textContent ?? "";
		expect(text).toContain("-");
	});

	it("Gutter mode shows only modified content with pure deletion", () => {
		const deletionProps = {
			originalContent: "line1\nremoved\nline2\n",
			modifiedContent: "line1\nline2\n",
			language: "typescript",
			hunks: [
				{
					index: 0,
					oldStart: 1,
					oldLines: 3,
					newStart: 1,
					newLines: 2,
					lines: [" line1", "-removed", " line2"],
				},
			],
		};
		const { container } = render(
			<ShikiDiffViewer {...deletionProps} diffMode="gutter" />,
		);
		const text = container.textContent ?? "";
		expect(text).not.toContain("removed");
		expect(text).toContain("-");
	});

	it("Inline mode displays both added and deleted line content", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="inline" />,
		);
		const text = container.textContent ?? "";
		expect(text).toContain("modified");
		expect(text).toContain("line2");
	});

	it("Gutter mode shows colored bar instead of full-line background", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="gutter" />,
		);
		const bars = container.querySelectorAll(".w-\\[4px\\]");
		expect(bars.length).toBeGreaterThan(0);
		const addedBars = container.querySelectorAll(
			".w-\\[4px\\].bg-\\[var\\(--status-added\\)\\]",
		);
		expect(addedBars.length).toBeGreaterThan(0);
	});

	it("Gutter mode deletion shows red bar", () => {
		const deletionProps = {
			originalContent: "line1\nremoved\nline2\n",
			modifiedContent: "line1\nline2\n",
			language: "typescript",
			hunks: [
				{
					index: 0,
					oldStart: 1,
					oldLines: 3,
					newStart: 1,
					newLines: 2,
					lines: [" line1", "-removed", " line2"],
				},
			],
		};
		const { container } = render(
			<ShikiDiffViewer {...deletionProps} diffMode="gutter" />,
		);
		const deletedBars = container.querySelectorAll(
			".w-\\[4px\\].bg-\\[var\\(--status-deleted\\)\\]",
		);
		expect(deletedBars.length).toBeGreaterThan(0);
	});

	it("renders scrollbar markers for diff blocks", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="inline" />,
		);
		const markersContainer = container.querySelector(
			"[data-testid='scrollbar-markers']",
		);
		expect(markersContainer).toBeDefined();
		expect(markersContainer?.getAttribute("aria-hidden")).toBe("true");
		const markers = markersContainer?.querySelectorAll("[class*='absolute']");
		expect(markers?.length).toBeGreaterThan(0);
	});

	it("does not render scrollbar markers when no diff blocks exist", () => {
		const noDiffProps = {
			originalContent: "same\n",
			modifiedContent: "same\n",
			language: "typescript",
			hunks: [] as typeof baseProps.hunks,
		};
		const { container } = render(
			<ShikiDiffViewer {...noDiffProps} diffMode="inline" />,
		);
		const markersContainer = container.querySelector(
			"[data-testid='scrollbar-markers']",
		);
		expect(markersContainer).toBeNull();
	});

	it("scrollbar markers rendered for all three modes", () => {
		for (const mode of ["gutter", "inline", "split"] as const) {
			const { container } = render(
				<ShikiDiffViewer {...baseProps} diffMode={mode} />,
			);
			const markersContainer = container.querySelector(
				"[data-testid='scrollbar-markers']",
			);
			expect(markersContainer).not.toBeNull();
		}
	});

	it("calls onStageGroup with correct groupIndex when stage button is clicked", () => {
		const onStage = vi.fn();
		const { container } = render(
			<ShikiDiffViewer
				{...baseProps}
				diffMode="inline"
				changeGroups={[
					{
						groupIndex: 2,
						hunkIndex: 0,
						newStart: 2,
						newEnd: 2,
						lineOffsetStart: 1,
						lineOffsetEnd: 3,
					},
				]}
				onStageGroup={onStage}
				groupActionLabel="Stage"
			/>,
		);
		const stageButton = container.querySelector(".hunk-stage") as HTMLElement;
		expect(stageButton).not.toBeNull();
		fireEvent.click(stageButton);
		expect(onStage).toHaveBeenCalledWith(2);
	});

	it("shows hidden lines banner when diffOnlyMode is true", async () => {
		const longOriginal =
			"line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n";
		const longModified =
			"line1\nline2\nline3\nline4\nchanged\nline6\nline7\nline8\nline9\nline10\n";
		const longHunks = [
			{
				index: 0,
				oldStart: 5,
				oldLines: 1,
				newStart: 5,
				newLines: 1,
				lines: ["-line5", "+changed"],
			},
		];
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue([
			{ startLine: 1, endLine: 2, hiddenCount: 2 },
		]);
		const { container } = render(
			<ShikiDiffViewer
				originalContent={longOriginal}
				modifiedContent={longModified}
				language="typescript"
				hunks={longHunks}
				diffMode="inline"
				diffOnlyMode={true}
			/>,
		);
		await waitFor(() => {
			expect(container.textContent).toContain("lines hidden");
		});
	});

	it("shows all lines without hidden banner when diffOnlyMode is false", () => {
		const { container } = render(
			<ShikiDiffViewer {...baseProps} diffMode="inline" diffOnlyMode={false} />,
		);
		expect(container.textContent).not.toContain("lines hidden");
		expect(container.textContent).toContain("line1");
		expect(container.textContent).toContain("line3");
	});
});
