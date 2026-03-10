import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { RightSidebarTop } from "./RightSidebarTop";

const defaultProps = {
	activeTab: "explorer" as const,
	onTabChange: vi.fn(),
	mode: "editor" as const,
	explorerContent: <div>Explorer Content</div>,
	changesContent: <div>Changes Content</div>,
	searchContent: <div>Search Content</div>,
	prContent: <div>PR Content</div>,
};

describe("RightSidebarTop", () => {
	it("editorモードでExplorer, Changes, Search, Symbols, Pull Requestsタブが表示される", () => {
		render(<RightSidebarTop {...defaultProps} mode="editor" />);

		expect(screen.getByRole("tab", { name: "Explorer" })).toBeInTheDocument();
		expect(screen.getByRole("tab", { name: "Changes" })).toBeInTheDocument();
		expect(screen.getByRole("tab", { name: "Search" })).toBeInTheDocument();
		expect(screen.getByRole("tab", { name: "Symbols" })).toBeInTheDocument();
		expect(
			screen.getByRole("tab", { name: "Pull Requests" }),
		).toBeInTheDocument();
	});

	it("editorモードでPlan Timeline, Plan Commentsタブが非表示", () => {
		render(<RightSidebarTop {...defaultProps} mode="editor" />);

		expect(
			screen.queryByRole("tab", { name: "Plan Timeline" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("tab", { name: "Plan Comments" }),
		).not.toBeInTheDocument();
	});

	it("workflowモードでPlan Timeline, Plan Commentsタブが表示される", () => {
		render(
			<RightSidebarTop
				{...defaultProps}
				mode="workflow"
				activeTab="plan-timeline"
			/>,
		);

		expect(
			screen.getByRole("tab", { name: "Plan Timeline" }),
		).toBeInTheDocument();
		expect(
			screen.getByRole("tab", { name: "Plan Comments" }),
		).toBeInTheDocument();
	});

	it("workflowモードでeditorモードのタブが非表示", () => {
		render(
			<RightSidebarTop
				{...defaultProps}
				mode="workflow"
				activeTab="plan-timeline"
			/>,
		);

		expect(
			screen.queryByRole("tab", { name: "Explorer" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("tab", { name: "Changes" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("tab", { name: "Search" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("tab", { name: "Symbols" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("tab", { name: "Pull Requests" }),
		).not.toBeInTheDocument();
	});

	it("workflowモードでplanTimelineContentがplan-timelineタブ選択時に描画される", () => {
		render(
			<RightSidebarTop
				{...defaultProps}
				mode="workflow"
				activeTab="plan-timeline"
				planTimelineContent={<div>Plan Timeline Content</div>}
			/>,
		);

		expect(screen.getByText("Plan Timeline Content")).toBeInTheDocument();
	});

	it("workflowモードでplanCommentContentがplan-commentタブ選択時に描画される", () => {
		render(
			<RightSidebarTop
				{...defaultProps}
				mode="workflow"
				activeTab="plan-comment"
				planCommentContent={<div>Plan Comment Content</div>}
			/>,
		);

		expect(screen.getByText("Plan Comment Content")).toBeInTheDocument();
	});

	it("workflowモードでeditorモードのコンテンツが描画されない", () => {
		render(
			<RightSidebarTop
				{...defaultProps}
				mode="workflow"
				activeTab="plan-timeline"
				planTimelineContent={<div>Plan Timeline Content</div>}
			/>,
		);

		expect(screen.queryByText("Explorer Content")).not.toBeInTheDocument();
		expect(screen.queryByText("Changes Content")).not.toBeInTheDocument();
		expect(screen.queryByText("Search Content")).not.toBeInTheDocument();
		expect(screen.queryByText("PR Content")).not.toBeInTheDocument();
	});

	it("タブクリックでonTabChangeが呼ばれる", async () => {
		const user = userEvent.setup();
		const onTabChange = vi.fn();

		render(
			<RightSidebarTop
				{...defaultProps}
				mode="editor"
				onTabChange={onTabChange}
			/>,
		);

		await user.click(screen.getByRole("tab", { name: "Changes" }));
		expect(onTabChange).toHaveBeenCalledWith("changes");
	});
});
