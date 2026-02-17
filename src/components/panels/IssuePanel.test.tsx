import { render, screen, waitFor } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { IssueInfo } from "@/types/git";
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

describe("IssuePanel", () => {
	const defaultProps = {
		repoPaths: ["/path/to/repo"],
		providerStatuses: { "/path/to/repo": "available" as const },
		onSelectWorktree: vi.fn(),
	};

	it("should render Issues header", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(mockIssues);

		render(<IssuePanel {...defaultProps} />);
		expect(screen.getByText("Issues")).toBeInTheDocument();
	});

	it("should display issue titles after loading", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(mockIssues);

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
		vi.mocked(invoke).mockResolvedValue(mockIssues);

		render(<IssuePanel {...defaultProps} />);
		await waitFor(() => {
			expect(screen.getByText("#305")).toBeInTheDocument();
		});
		expect(screen.getByText("#100")).toBeInTheDocument();
	});

	it("should display labels with color", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(mockIssues);

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
		vi.mocked(invoke).mockResolvedValue(mockIssues);

		render(<IssuePanel {...defaultProps} />);
		await waitFor(() => {
			expect(screen.getByText("user1")).toBeInTheDocument();
		});
	});

	it("should show Create Worktree buttons", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(mockIssues);

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
		vi.mocked(invoke).mockResolvedValue([]);

		render(<IssuePanel {...defaultProps} />);
		await waitFor(() => {
			expect(screen.getByText("No open issues")).toBeInTheDocument();
		});
	});

	it("should collapse and expand repo section on click", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(mockIssues);
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
		vi.mocked(invoke).mockResolvedValue(mockIssues);

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
		vi.mocked(invoke).mockResolvedValue(mockIssues);
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
		vi.mocked(invoke).mockResolvedValue(mockIssues);
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
		vi.mocked(invoke).mockResolvedValue(mockIssues);
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
});
