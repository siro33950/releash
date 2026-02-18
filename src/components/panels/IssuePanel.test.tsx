import { render, screen, waitFor } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { IssueInfo, WorktreeEntry } from "@/types/git";
import { IssuePanel } from "./IssuePanel";

const mockIssues: IssueInfo[] = [
	{
		number: 305,
		title: "Kanban画面にIssue管理パネルを追加",
		state: "OPEN",
		url: "https://github.com/owner/repo/issues/305",
		author: { login: "user1" },
		created_at: "2024-01-01T00:00:00Z",
		updated_at: "2024-01-02T00:00:00Z",
		labels: [{ name: "enhancement", color: "a2eeef" }],
		assignees: [{ login: "user1" }],
		body: "Issue body",
		milestone: null,
	},
	{
		number: 100,
		title: "Bug fix",
		state: "OPEN",
		url: "https://github.com/owner/repo/issues/100",
		author: { login: "user2" },
		created_at: "2024-06-15T00:00:00Z",
		updated_at: "2024-06-15T00:00:00Z",
		labels: [],
		assignees: [],
		body: "",
		milestone: null,
	},
];

function mockInvokeDefault(command: string) {
	if (command === "get_cached_issues") return Promise.resolve(mockIssues);
	if (command === "list_worktrees") return Promise.resolve([]);
	return Promise.resolve(null);
}

describe("IssuePanel", () => {
	const defaultProps = {
		repoPaths: ["/path/to/repo"],
		providerStatuses: { "/path/to/repo": "available" as const },
		onSelectWorktree: vi.fn(),
	};

	it("should render Issues header", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);

		render(<IssuePanel {...defaultProps} />);
		expect(screen.getByText("Issues")).toBeInTheDocument();
	});

	it("should display issue titles after loading", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);

		render(<IssuePanel {...defaultProps} />);
		await waitFor(() => {
			expect(
				screen.getByText("Kanban画面にIssue管理パネルを追加"),
			).toBeInTheDocument();
		});
		expect(screen.getByText("Bug fix")).toBeInTheDocument();
	});

	it("should display issue numbers", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);

		render(<IssuePanel {...defaultProps} />);
		await waitFor(() => {
			expect(screen.getByText("#305")).toBeInTheDocument();
		});
		expect(screen.getByText("#100")).toBeInTheDocument();
	});

	it("should display labels with color", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);

		render(<IssuePanel {...defaultProps} />);
		await waitFor(() => {
			const labels = screen.getAllByText("enhancement");
			expect(labels.length).toBeGreaterThanOrEqual(1);
			const badge = labels.find((el) => el.tagName === "SPAN");
			expect(badge).toBeDefined();
		});
	});

	it("should display assignees", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);

		render(<IssuePanel {...defaultProps} />);
		await waitFor(() => {
			expect(screen.getByText("user1")).toBeInTheDocument();
		});
	});

	it("should show Create Worktree buttons", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);

		render(<IssuePanel {...defaultProps} />);
		await waitFor(() => {
			const buttons = screen.getAllByText("Create Worktree");
			expect(buttons).toHaveLength(2);
		});
	});

	it("should show no repositories message when repoPaths is empty", () => {
		render(
			<IssuePanel
				repoPaths={[]}
				providerStatuses={{}}
				onSelectWorktree={vi.fn()}
			/>,
		);
		expect(screen.getByText("No repositories")).toBeInTheDocument();
	});

	it("should show no open issues when issue list is empty", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((command: string) => {
			if (command === "get_cached_issues") return Promise.resolve([]);
			if (command === "list_worktrees") return Promise.resolve([]);
			return Promise.resolve(null);
		});

		render(<IssuePanel {...defaultProps} />);
		await waitFor(() => {
			expect(screen.getByText("No open issues")).toBeInTheDocument();
		});
	});

	it("should collapse and expand repo section on click", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);
		const user = userEvent.setup();

		render(<IssuePanel {...defaultProps} />);

		await waitFor(() => {
			expect(
				screen.getByText("Kanban画面にIssue管理パネルを追加"),
			).toBeInTheDocument();
		});

		const repoButton = screen.getByText("repo");
		await user.click(repoButton);

		expect(
			screen.queryByText("Kanban画面にIssue管理パネルを追加"),
		).not.toBeInTheDocument();

		await user.click(repoButton);

		await waitFor(() => {
			expect(
				screen.getByText("Kanban画面にIssue管理パネルを追加"),
			).toBeInTheDocument();
		});
	});

	it("should sort issues by number descending", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);

		render(<IssuePanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("#305")).toBeInTheDocument();
		});

		const cards = screen.getAllByText(/^#\d+$/);
		expect(cards[0].textContent).toBe("#305");
		expect(cards[1].textContent).toBe("#100");
	});

	it("should filter issues by title prefix", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);
		const user = userEvent.setup();

		render(<IssuePanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("#305")).toBeInTheDocument();
		});

		const filterInput = screen.getByPlaceholderText("Filter by title...");
		await user.type(filterInput, "Bug");

		expect(screen.getByText("Bug fix")).toBeInTheDocument();
		expect(
			screen.queryByText("Kanban画面にIssue管理パネルを追加"),
		).not.toBeInTheDocument();
	});

	it("should show no matching issues when filter has no matches", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);
		const user = userEvent.setup();

		render(<IssuePanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("#305")).toBeInTheDocument();
		});

		const filterInput = screen.getByPlaceholderText("Filter by title...");
		await user.type(filterInput, "zzz");

		expect(screen.getByText("No matching issues")).toBeInTheDocument();
	});

	it("should filter issues by label", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation(mockInvokeDefault as never);
		const user = userEvent.setup();

		render(<IssuePanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("#305")).toBeInTheDocument();
		});

		const labelSelect = screen.getByDisplayValue("All labels");
		await user.selectOptions(labelSelect, "enhancement");

		expect(
			screen.getByText("Kanban画面にIssue管理パネルを追加"),
		).toBeInTheDocument();
		expect(screen.queryByText("Bug fix")).not.toBeInTheDocument();
	});

	it("should show Open Worktree button when worktree exists for an issue", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockWorktrees: WorktreeEntry[] = [
			{
				name: "feat-issues-305",
				path: "/worktrees/feat-issues-305",
				branch: "feat/issues/305",
				is_main: false,
				is_locked: false,
				dirty_count: 0,
				base_branch: "main",
			},
		];
		vi.mocked(invoke).mockImplementation((command: string) => {
			if (command === "get_cached_issues") return Promise.resolve(mockIssues);
			if (command === "list_worktrees") return Promise.resolve(mockWorktrees);
			return Promise.resolve(null);
		});

		render(<IssuePanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Open Worktree")).toBeInTheDocument();
		});
		const createButtons = screen.getAllByText("Create Worktree");
		expect(createButtons).toHaveLength(1);
	});

	it("should call onSelectWorktree with existing worktree path when Open Worktree is clicked", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const onSelectWorktree = vi.fn();
		const mockWorktrees: WorktreeEntry[] = [
			{
				name: "feat-issues-305",
				path: "/worktrees/feat-issues-305",
				branch: "feat/issues/305",
				is_main: false,
				is_locked: false,
				dirty_count: 0,
				base_branch: "main",
			},
		];
		vi.mocked(invoke).mockImplementation((command: string) => {
			if (command === "get_cached_issues") return Promise.resolve(mockIssues);
			if (command === "list_worktrees") return Promise.resolve(mockWorktrees);
			return Promise.resolve(null);
		});
		const user = userEvent.setup();

		render(
			<IssuePanel
				repoPaths={["/path/to/repo"]}
				providerStatuses={{ "/path/to/repo": "available" }}
				onSelectWorktree={onSelectWorktree}
			/>,
		);

		await waitFor(() => {
			expect(screen.getByText("Open Worktree")).toBeInTheDocument();
		});

		await user.click(screen.getByText("Open Worktree"));

		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/worktrees/feat-issues-305",
			"feat/issues/305",
			"repo",
		);
	});

	it("should re-fetch worktrees after Create Worktree succeeds", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const createdEntry: WorktreeEntry = {
			name: "feat-issues-100",
			path: "/worktrees/feat-issues-100",
			branch: "feat/issues/100",
			is_main: false,
			is_locked: false,
			dirty_count: 0,
			base_branch: "main",
		};
		let listWorktreeCallCount = 0;
		vi.mocked(invoke).mockImplementation((command: string) => {
			if (command === "get_cached_issues") return Promise.resolve(mockIssues);
			if (command === "list_worktrees") {
				listWorktreeCallCount++;
				if (listWorktreeCallCount >= 2) {
					return Promise.resolve([createdEntry]);
				}
				return Promise.resolve([]);
			}
			if (command === "get_default_branch") return Promise.resolve("main");
			if (command === "create_worktree") return Promise.resolve(createdEntry);
			return Promise.resolve(null);
		});
		const user = userEvent.setup();

		render(<IssuePanel {...defaultProps} />);

		await waitFor(() => {
			const buttons = screen.getAllByText("Create Worktree");
			expect(buttons).toHaveLength(2);
		});

		const createButtons = screen.getAllByText("Create Worktree");
		await user.click(createButtons[1]);

		await waitFor(() => {
			expect(listWorktreeCallCount).toBeGreaterThanOrEqual(2);
		});
	});
});
