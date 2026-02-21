import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BranchCard } from "@/types/git";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const mockListen = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

import { RepoKanbanBoard } from "./RepoKanbanBoard";

const todoBranch: BranchCard = {
	name: "feat/todo",
	is_default: false,
	worktree_path: null,
	dirty_count: 0,
	is_merged: false,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	is_remote_only: false,
	has_upstream: true,
	remote_name: null,
};

function setupMocks(branches: BranchCard[] = []) {
	mockListen.mockResolvedValue(() => {});
	mockInvoke.mockImplementation((cmd: string) => {
		switch (cmd) {
			case "list_branches_with_status":
				return Promise.resolve(branches);
			case "get_cached_pr_status":
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			case "get_agent_states":
				return Promise.resolve({});
			case "get_releash_base":
				return Promise.resolve(null);
			case "get_default_branch":
				return Promise.resolve("main");
			default:
				return Promise.resolve(null);
		}
	});
}

const defaultProps = {
	repoPath: "/tmp/repo",
	providerStatus: null,
	onSelectWorktree: vi.fn(),
	onRemove: vi.fn(),
};

describe("RepoKanbanBoard", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should show empty state when no branches exist", async () => {
		setupMocks([]);
		render(<RepoKanbanBoard {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("No worktrees")).toBeInTheDocument();
		});
		expect(
			screen.getByText("Create a branch to start working with worktrees"),
		).toBeInTheDocument();
		expect(screen.getByRole("button", { name: "New" })).toBeInTheDocument();
	});

	it("should show kanban columns when branches exist", async () => {
		setupMocks([todoBranch]);
		render(<RepoKanbanBoard {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Todo")).toBeInTheDocument();
		});
		expect(screen.getByText("In Progress")).toBeInTheDocument();
		expect(screen.getByText("Review")).toBeInTheDocument();
		expect(screen.getByText("Done")).toBeInTheDocument();
		expect(screen.queryByText("No worktrees")).not.toBeInTheDocument();
	});

	it("should open create dialog when empty state button is clicked", async () => {
		setupMocks([]);
		const user = userEvent.setup();
		render(<RepoKanbanBoard {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByRole("button", { name: "New" })).toBeInTheDocument();
		});

		await user.click(screen.getByRole("button", { name: "New" }));

		await waitFor(() => {
			expect(screen.getByText("New Workspace")).toBeInTheDocument();
		});
	});
});
