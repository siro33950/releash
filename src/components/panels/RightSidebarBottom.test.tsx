import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AppSettings } from "@/types/settings";
import { RightSidebarBottom } from "./RightSidebarBottom";

vi.mock("@/components/panels/TerminalPanel", () => ({
	TerminalPanel: vi.fn(() => <div data-testid="terminal-panel" />),
}));

vi.mock("@/components/panels/CommentList", () => ({
	CommentList: vi.fn(() => <div data-testid="comment-list" />),
}));

vi.mock("@/hooks/useReviewExecution", () => ({
	useReviewExecution: vi.fn(() => ({
		status: "idle",
		summary: null,
		progress: null,
		fileStates: [],
		startReview: vi.fn(),
		cancelReview: vi.fn(),
		reset: vi.fn(),
	})),
}));

vi.mock("@/lib/formatCommentsForTerminal", () => ({
	formatCommentsForTerminal: vi.fn(() => ""),
}));

const defaultSettings: AppSettings = {
	reviewAgent: "none",
	aiModel: "gpt-4",
	terminalFont: "monospace",
	terminalFontSize: 14,
	uiLanguage: "en",
};

const defaultProps = {
	rootPath: "/test",
	settings: defaultSettings,
	threads: [],
	mode: "editor" as const,
};

describe("RightSidebarBottom", () => {
	it("workflowモードでTimelineタブが表示される", () => {
		render(<RightSidebarBottom {...defaultProps} mode="workflow" />);

		expect(screen.getByRole("tab", { name: "Timeline" })).toBeInTheDocument();
	});

	it("editorモードでTimelineタブが非表示", () => {
		render(<RightSidebarBottom {...defaultProps} mode="editor" />);

		expect(
			screen.queryByRole("tab", { name: "Timeline" }),
		).not.toBeInTheDocument();
	});

	it("workflowモードでimplTimelineContentがtimelineタブ選択時に描画される", () => {
		render(
			<RightSidebarBottom
				{...defaultProps}
				mode="workflow"
				activeTab="timeline"
				implTimelineContent={<div>Impl Timeline Content</div>}
			/>,
		);

		expect(screen.getByText("Impl Timeline Content")).toBeInTheDocument();
	});

	it("TerminalタブとCommentsタブは両モードで表示される", () => {
		const { rerender } = render(
			<RightSidebarBottom {...defaultProps} mode="editor" />,
		);

		expect(screen.getByRole("tab", { name: "Terminal" })).toBeInTheDocument();
		expect(screen.getByRole("tab", { name: "Comments" })).toBeInTheDocument();

		rerender(<RightSidebarBottom {...defaultProps} mode="workflow" />);

		expect(screen.getByRole("tab", { name: "Terminal" })).toBeInTheDocument();
		expect(screen.getByRole("tab", { name: "Comments" })).toBeInTheDocument();
	});
});
