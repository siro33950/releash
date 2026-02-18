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
			},
			{
				property_name: "Tags",
				property_type: "multi_select",
				options: ["frontend", "backend"],
			},
		];

		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "query_notion_tasks") return mockTaskPage;
			if (cmd === "fetch_notion_label_options") return mockLabelOptions;
			return null;
		});

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Implement feature")).toBeInTheDocument();
		});

		expect(screen.getAllByText("In Progress").length).toBeGreaterThanOrEqual(1);
		expect(screen.getAllByText("frontend").length).toBeGreaterThanOrEqual(1);
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
			},
		];

		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return mockConfig;
			if (cmd === "query_notion_tasks") return mockTaskPage;
			if (cmd === "fetch_notion_label_options") return mockLabelOptions;
			return null;
		});

		render(<NotionPanel {...defaultProps} />);

		await waitFor(() => {
			expect(screen.getByText("Task A")).toBeInTheDocument();
		});

		expect(screen.getByDisplayValue("Status: All")).toBeInTheDocument();
	});
});
