import { render, screen, waitFor } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { NotionPanel } from "./NotionPanel";

describe("NotionPanel", () => {
	const defaultProps = {
		repoPaths: ["/path/to/repo"],
		onSelectWorktree: vi.fn(),
	};

	it("should render Notion Tasks header", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		render(<NotionPanel {...defaultProps} />);
		expect(screen.getByText("Notion Tasks")).toBeInTheDocument();
	});

	it("should show no repositories message when repoPaths is empty", () => {
		render(<NotionPanel repoPaths={[]} onSelectWorktree={vi.fn()} />);
		expect(screen.getByText("No repositories")).toBeInTheDocument();
	});

	it("should show unconfigured message when no config exists", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Notion連携が未設定です")).toBeInTheDocument();
		});
	});

	it("should show config form when clicking setup button", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);
		const user = userEvent.setup();

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("設定する")).toBeInTheDocument();
		});

		await user.click(screen.getByText("設定する"));

		expect(screen.getByText("API Token")).toBeInTheDocument();
		expect(screen.getByText("Database ID")).toBeInTheDocument();
	});

	it("should show task list when configured", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockConfig = {
			api_token: "ntn_test",
			database_id: "db-123",
			property_mapping: {
				title: "Name",
				labels: [
					{ name: "Status", property_type: "status" },
					{ name: "Tags", property_type: "multi_select" },
				],
				branch_name: "",
				branch_prefix: "",
			},
		};
		const mockTaskPage = {
			tasks: [
				{
					id: "page-1",
					title: "Implement feature",
					url: "https://notion.so/page-1",
					labels: {
						Status: ["In Progress"],
						Tags: ["frontend"],
					},
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-02T00:00:00.000Z",
				},
			],
			has_more: false,
			next_cursor: null,
		};
		const mockLabelOptions = [
			{
				property_name: "Status",
				property_type: "status",
				options: ["Todo", "In Progress", "Done"],
				option_ids: [],
			},
			{
				property_name: "Tags",
				property_type: "multi_select",
				options: ["frontend", "backend"],
				option_ids: [],
			},
		];

		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "query_notion_tasks") return mockTaskPage;
			if (cmd === "fetch_notion_label_options") return mockLabelOptions;
			if (cmd === "list_worktrees") return [];
			return null;
		});

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Implement feature")).toBeInTheDocument();
		});

		expect(screen.getAllByText("In Progress").length).toBeGreaterThanOrEqual(1);
		expect(screen.getAllByText("frontend").length).toBeGreaterThanOrEqual(1);
	});

	it("should display people properties as badges", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockConfig = {
			api_token: "ntn_test",
			database_id: "db-123",
			property_mapping: {
				title: "Name",
				labels: [
					{ name: "Status", property_type: "status" },
					{ name: "Assignee", property_type: "people" },
				],
				branch_name: "",
				branch_prefix: "",
			},
		};
		const mockTaskPage = {
			tasks: [
				{
					id: "page-p1",
					title: "People task",
					url: "https://notion.so/page-p1",
					labels: {
						Status: ["In Progress"],
						Assignee: ["Alice", "Bob"],
					},
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-02T00:00:00.000Z",
				},
			],
			has_more: false,
			next_cursor: null,
		};
		const mockLabelOptions = [
			{
				property_name: "Status",
				property_type: "status",
				options: ["Todo", "In Progress", "Done"],
				option_ids: [],
			},
			{
				property_name: "Assignee",
				property_type: "people",
				options: ["Alice", "Bob"],
				option_ids: ["uuid-1", "uuid-2"],
			},
		];

		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "query_notion_tasks") return mockTaskPage;
			if (cmd === "fetch_notion_label_options") return mockLabelOptions;
			if (cmd === "list_worktrees") return [];
			return null;
		});

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("People task")).toBeInTheDocument();
		});

		expect(screen.getAllByText("Alice").length).toBeGreaterThanOrEqual(1);
		expect(screen.getAllByText("Bob").length).toBeGreaterThanOrEqual(1);
		expect(screen.getAllByText("In Progress").length).toBeGreaterThanOrEqual(1);
	});

	it("should show Create Worktree button for each task", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockConfig = {
			api_token: "ntn_test",
			database_id: "db-123",
			property_mapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		};
		const mockTaskPage = {
			tasks: [
				{
					id: "page-1",
					title: "Task 1",
					url: "https://notion.so/page-1",
					labels: {},
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-01T00:00:00.000Z",
				},
				{
					id: "page-2",
					title: "Task 2",
					url: "https://notion.so/page-2",
					labels: {},
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-01T00:00:00.000Z",
				},
			],
			has_more: false,
			next_cursor: null,
		};

		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "query_notion_tasks") return mockTaskPage;
			if (cmd === "fetch_notion_label_options") return [];
			if (cmd === "list_worktrees") return [];
			return null;
		});

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			const buttons = screen.getAllByText("Create Worktree");
			expect(buttons).toHaveLength(2);
		});
	});

	it("should show Load more button when hasMore is true", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockConfig = {
			api_token: "ntn_test",
			database_id: "db-123",
			property_mapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		};
		const mockTaskPage = {
			tasks: [
				{
					id: "page-1",
					title: "Task 1",
					url: "https://notion.so/page-1",
					labels: {},
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-01T00:00:00.000Z",
				},
			],
			has_more: true,
			next_cursor: "cursor-abc",
		};

		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "query_notion_tasks") return mockTaskPage;
			if (cmd === "fetch_notion_label_options") return [];
			if (cmd === "list_worktrees") return [];
			return null;
		});

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Load more")).toBeInTheDocument();
		});
	});

	it("should collapse and expand repo section on click", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);
		const user = userEvent.setup();

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Notion連携が未設定です")).toBeInTheDocument();
		});

		const repoButton = screen.getByText("repo");
		await user.click(repoButton);

		expect(
			screen.queryByText("Notion連携が未設定です"),
		).not.toBeInTheDocument();

		await user.click(repoButton);

		await waitFor(() => {
			expect(screen.getByText("Notion連携が未設定です")).toBeInTheDocument();
		});
	});

	it("should render label filter dropdowns from server options", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockConfig = {
			api_token: "ntn_test",
			database_id: "db-123",
			property_mapping: {
				title: "Name",
				labels: [
					{ name: "Status", property_type: "status" },
					{ name: "Tags", property_type: "multi_select" },
				],
				branch_name: "",
				branch_prefix: "",
			},
		};
		const mockTaskPage = {
			tasks: [
				{
					id: "page-1",
					title: "Task A",
					url: "https://notion.so/page-1",
					labels: { Status: ["Todo"] },
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-01T00:00:00.000Z",
				},
			],
			has_more: false,
			next_cursor: null,
		};
		const mockLabelOptions = [
			{
				property_name: "Status",
				property_type: "status",
				options: ["Todo", "In Progress", "Done"],
				option_ids: [],
			},
		];

		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "query_notion_tasks") return mockTaskPage;
			if (cmd === "fetch_notion_label_options") return mockLabelOptions;
			if (cmd === "list_worktrees") return [];
			return null;
		});

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Task A")).toBeInTheDocument();
		});

		expect(screen.getByDisplayValue("Status: All")).toBeInTheDocument();
	});

	it("should show branch prefix input in config form", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockConfig = {
			api_token: "ntn_test",
			database_id: "db-123",
			property_mapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "feat/",
			},
		};
		const mockValidation = {
			status: "configured",
			properties: [
				{ name: "Name", property_type: "title", options: [] },
				{ name: "Branch", property_type: "rich_text", options: [] },
			],
		};

		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "validate_notion_config") return mockValidation;
			if (cmd === "query_notion_tasks")
				return { tasks: [], has_more: false, next_cursor: null };
			if (cmd === "fetch_notion_label_options") return [];
			if (cmd === "list_worktrees") return [];
			return null;
		});

		const user = userEvent.setup();
		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("設定")).toBeInTheDocument();
		});

		await user.click(screen.getByText("設定"));

		await waitFor(() => {
			expect(screen.getByText("API Token")).toBeInTheDocument();
		});

		await user.click(screen.getByText("接続テスト"));

		await waitFor(() => {
			expect(screen.getByText("プレフィックス")).toBeInTheDocument();
		});

		const prefixInput = screen.getByPlaceholderText("feat/");
		expect(prefixInput).toBeInTheDocument();
		expect(prefixInput).toHaveValue("feat/");
	});

	it("should show Open Worktree when worktree already exists for a task", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockConfig = {
			api_token: "ntn_test",
			database_id: "db-123",
			property_mapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		};
		const mockTaskPage = {
			tasks: [
				{
					id: "page-1",
					title: "Existing task",
					url: "https://notion.so/page-1",
					labels: {},
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-01T00:00:00.000Z",
				},
			],
			has_more: false,
			next_cursor: null,
		};
		const mockWorktrees = [
			{
				path: "/worktrees/Existing-task-page1",
				branch: "Existing-task",
				is_main: false,
			},
		];

		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "query_notion_tasks") return mockTaskPage;
			if (cmd === "fetch_notion_label_options") return [];
			if (cmd === "list_worktrees") return mockWorktrees;
			return null;
		});

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Open Worktree")).toBeInTheDocument();
		});
		expect(screen.queryByText("Create Worktree")).not.toBeInTheDocument();
	});

	it("should refresh worktree list after creating a worktree", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockConfig = {
			api_token: "ntn_test",
			database_id: "db-123",
			property_mapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		};
		const mockTaskPage = {
			tasks: [
				{
					id: "page-1",
					title: "New task",
					url: "https://notion.so/page-1",
					labels: {},
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-01T00:00:00.000Z",
				},
			],
			has_more: false,
			next_cursor: null,
		};

		let worktreeCallCount = 0;
		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "query_notion_tasks") return mockTaskPage;
			if (cmd === "fetch_notion_label_options") return [];
			if (cmd === "list_worktrees") {
				worktreeCallCount++;
				if (worktreeCallCount >= 2) {
					return [
						{
							path: "/worktrees/New-task",
							branch: "New-task",
							is_main: false,
						},
					];
				}
				return [];
			}
			if (cmd === "get_default_branch") return "main";
			if (cmd === "create_worktree")
				return {
					path: "/worktrees/New-task",
					branch: "New-task",
					is_main: false,
				};
			return null;
		});

		const user = userEvent.setup();
		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Create Worktree")).toBeInTheDocument();
		});

		await user.click(screen.getByText("Create Worktree"));

		await waitFor(() => {
			expect(screen.getByText("Open Worktree")).toBeInTheDocument();
		});
	});
});
