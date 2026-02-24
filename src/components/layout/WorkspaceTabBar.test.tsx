import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WorkspaceTab } from "@/types/workspace-tab";
import { WorkspaceTabBar } from "./WorkspaceTabBar";

const kanbanTab: WorkspaceTab = { type: "kanban", id: "kanban" };
const worktreeTab: WorkspaceTab = {
	type: "worktree",
	id: "/path/a",
	rootPath: "/path/a",
	branchName: "feat/login",
};

describe("WorkspaceTabBar", () => {
	it("should render kanban as a fixed button without close button", () => {
		render(
			<WorkspaceTabBar
				tabs={[kanbanTab]}
				activeTabId="kanban"
				onTabClick={vi.fn()}
				onTabClose={vi.fn()}
			/>,
		);
		screen.getByRole("button", { name: /Kanban/i });
		expect(screen.queryByLabelText(/Close/)).not.toBeInTheDocument();
	});

	it("should render worktree tab with branch name and close button", () => {
		render(
			<WorkspaceTabBar
				tabs={[kanbanTab, worktreeTab]}
				activeTabId="/path/a"
				onTabClick={vi.fn()}
				onTabClose={vi.fn()}
			/>,
		);
		expect(screen.getByText("feat/login")).toBeInTheDocument();
		expect(screen.getByLabelText("Close feat/login")).toBeInTheDocument();
	});

	it("should call onTabClick when tab is clicked", () => {
		const onTabClick = vi.fn();
		render(
			<WorkspaceTabBar
				tabs={[kanbanTab, worktreeTab]}
				activeTabId="kanban"
				onTabClick={onTabClick}
				onTabClose={vi.fn()}
			/>,
		);
		fireEvent.click(screen.getByText("feat/login"));
		expect(onTabClick).toHaveBeenCalledWith("/path/a");
	});

	it("should call onTabClose when close button is clicked", () => {
		const onTabClose = vi.fn();
		render(
			<WorkspaceTabBar
				tabs={[kanbanTab, worktreeTab]}
				activeTabId="/path/a"
				onTabClick={vi.fn()}
				onTabClose={onTabClose}
			/>,
		);
		fireEvent.click(screen.getByLabelText("Close feat/login"));
		expect(onTabClose).toHaveBeenCalledWith("/path/a");
	});

	it("should display agent state dot on worktree tab", () => {
		const tabWithAgent: WorkspaceTab = {
			...worktreeTab,
			agentState: "running",
		};
		render(
			<WorkspaceTabBar
				tabs={[kanbanTab, tabWithAgent]}
				activeTabId="kanban"
				onTabClick={vi.fn()}
				onTabClose={vi.fn()}
			/>,
		);
		expect(screen.getByTitle("running")).toBeInTheDocument();
	});

	it("should mark active tab with aria-selected", () => {
		const secondWorktree: WorkspaceTab = {
			type: "worktree",
			id: "/path/b",
			rootPath: "/path/b",
			branchName: "fix/bug",
		};
		render(
			<WorkspaceTabBar
				tabs={[kanbanTab, worktreeTab, secondWorktree]}
				activeTabId="/path/b"
				onTabClick={vi.fn()}
				onTabClose={vi.fn()}
			/>,
		);
		const tabs = screen.getAllByRole("tab");
		expect(tabs[0]).toHaveAttribute("aria-selected", "false");
		expect(tabs[1]).toHaveAttribute("aria-selected", "true");
	});
});
