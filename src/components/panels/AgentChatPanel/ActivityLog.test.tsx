import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ActivityEntry, MessagePart } from "@/types/session";
import {
	ActivityItem,
	resetActivityLogUiStateForTest,
	TaskToolActivity,
	ToolActivity,
} from "./ActivityLog";
import { resetAgentEditPreviewPanelStateForTest } from "./AgentEditPreviewPanel";
import type { TaskGroup } from "./toolPairing";

const mockInvoke = vi.fn().mockResolvedValue(null);
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));
vi.mock("../DiffViewerSection", () => ({
	DiffViewerSection: ({
		originalContent,
		modifiedContent,
	}: {
		originalContent: string;
		modifiedContent: string;
	}) => (
		<div data-testid="agent-diff-preview">
			<pre>{originalContent}</pre>
			<pre>{modifiedContent}</pre>
		</div>
	),
}));

function presentToolActivity(args: {
	toolName: string;
	input: Record<string, unknown>;
	basePath?: string;
}) {
	const { toolName, input, basePath } = args;
	const shorten = (value: string) => {
		if (!basePath) return value;
		if (value === basePath) return ".";
		return value.startsWith(`${basePath}/`)
			? value.slice(basePath.length + 1)
			: value;
	};
	const truncate = (value: string, max: number) =>
		value.length > max ? `${value.slice(0, max)}...` : value;
	const mcpRead =
		toolName.startsWith("mcp__") &&
		/(read|get|list|search|fetch|retrieve|query)/.test(toolName.toLowerCase());
	const mcpWrite =
		toolName.startsWith("mcp__") &&
		/(write|create|update|delete|edit|post|patch|put)/.test(
			toolName.toLowerCase(),
		);
	const category = toolName.startsWith("mcp__")
		? "mcp"
		: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"].includes(toolName) ||
				mcpRead
			? "read"
			: toolName === "Bash"
				? "command"
				: ["Write", "Edit", "NotebookEdit"].includes(toolName) || mcpWrite
					? "write"
					: "other";
	const summary =
		typeof input.file_path === "string"
			? shorten(input.file_path)
			: typeof input.pattern === "string"
				? input.pattern
				: typeof input.command === "string"
					? truncate(input.command, 80)
					: Object.keys(input).length === 0
						? toolName
						: typeof input[Object.keys(input)[0]] === "string"
							? truncate(input[Object.keys(input)[0]] as string, 60)
							: `${Object.keys(input)[0]}: ...`;
	const label =
		category === "read"
			? typeof input.file_path === "string"
				? `Explored ${shorten(input.file_path)}`
				: typeof input.pattern === "string"
					? `Explored ${input.pattern}`
					: typeof input.path === "string"
						? `Explored ${shorten(input.path)}`
						: typeof input.query === "string"
							? `Searched "${truncate(input.query, 60)}"`
							: typeof input.url === "string"
								? `Fetched ${input.url}`
								: `Explored (${toolName})`
			: category === "command"
				? typeof input.command === "string"
					? input.command
					: "command"
				: category === "mcp"
					? (() => {
							const [, server = "server", name = "tool"] = toolName.split("__");
							return `${server}/${name}`;
						})()
					: summary === toolName
						? toolName
						: `${toolName} ${summary}`;
	return {
		category,
		label,
		summary,
		editPreviewTool: ["Edit", "MultiEdit", "Write"].includes(toolName),
	};
}

const clipboardWriteText = vi.fn().mockResolvedValue(undefined);
Object.defineProperty(navigator, "clipboard", {
	configurable: true,
	value: {
		writeText: clipboardWriteText,
	},
});

beforeEach(() => {
	resetActivityLogUiStateForTest();
	resetAgentEditPreviewPanelStateForTest();
	mockInvoke.mockClear();
	mockInvoke.mockImplementation((command: string, args: unknown) => {
		if (command === "present_agent_tool_activity") {
			return Promise.resolve(
				presentToolActivity(
					args as {
						toolName: string;
						input: Record<string, unknown>;
						basePath?: string;
					},
				),
			);
		}
		if (command === "get_language_from_path") {
			return Promise.resolve("typescript");
		}
		if (command === "compute_diff_hunks") {
			return Promise.resolve({
				hunks: [
					{
						index: 0,
						oldStart: 1,
						oldLines: 1,
						newStart: 1,
						newLines: 1,
						lines: ["-old", "+new"],
					},
				],
			});
		}
		if (command === "build_agent_edit_preview") {
			const input = (
				args as {
					input?: Record<string, unknown>;
				}
			)?.input;
			const filePath =
				typeof input?.file_path === "string" ? input.file_path : "src/app.ts";
			return Promise.resolve({
				toolName: "Edit",
				operation: "Edit first match",
				filePath,
				originalContent: "old",
				modifiedContent: "new",
				hunks: [
					{
						oldStart: 1,
						newStart: 1,
						lines: [
							{
								kind: "removed",
								oldLine: 1,
								newLine: null,
								content: "old",
							},
							{
								kind: "added",
								oldLine: null,
								newLine: 1,
								content: "new",
							},
						],
					},
				],
				warnings: [],
			});
		}
		return Promise.resolve(null);
	});
	clipboardWriteText.mockClear();
});

describe("ToolActivity", () => {
	describe("ReadToolActivity", () => {
		it("shows collapsed read tool with label", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} />);

			await waitFor(() =>
				expect(screen.getByTestId("activity-tool-use-0")).toHaveTextContent(
					"Explored /src/main.ts",
				),
			);
		});

		it("expands to show result content on click", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t1",
			};
			const result = {
				type: "tool_result" as const,
				content: "file contents here",
				isError: false,
			};
			render(<ToolActivity entry={entry} result={result} index={0} />);

			expect(screen.queryByText("file contents here")).toBeNull();

			fireEvent.click(await screen.findByText("Explored /src/main.ts"));
			expect(screen.getByText("file contents here")).toBeInTheDocument();
		});

		it("shows Glob tool with pattern label", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Glob",
				input: { pattern: "**/*.ts" },
				id: "t2",
			};
			render(<ToolActivity entry={entry} index={0} />);
			await waitFor(() =>
				expect(screen.getByTestId("activity-tool-use-0")).toHaveTextContent(
					"Explored **/*.ts",
				),
			);
		});

		it("does not re-fetch presentation when semantic input is unchanged", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/stable.ts" },
				id: "t-stable-presentation",
			};
			const { rerender } = render(<ToolActivity entry={entry} index={0} />);

			await waitFor(() =>
				expect(mockInvoke).toHaveBeenCalledWith(
					"present_agent_tool_activity",
					expect.objectContaining({
						toolName: "Read",
						input: { file_path: "/src/stable.ts" },
					}),
				),
			);
			const presentCalls = mockInvoke.mock.calls.filter(
				([command]) => command === "present_agent_tool_activity",
			);
			expect(presentCalls).toHaveLength(1);

			rerender(
				<ToolActivity
					entry={{
						...entry,
						input: { file_path: "/src/stable.ts" },
					}}
					index={0}
				/>,
			);

			await waitFor(() => {
				const nextPresentCalls = mockInvoke.mock.calls.filter(
					([command]) => command === "present_agent_tool_activity",
				);
				expect(nextPresentCalls).toHaveLength(1);
			});
		});

		it("keeps expanded content open after remounting the same tool use", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/persist.ts" },
				id: "t-persist-expanded",
			};
			const result = {
				type: "tool_result" as const,
				content: "persistent file contents",
				isError: false,
			};
			const { unmount } = render(
				<ToolActivity entry={entry} result={result} index={0} />,
			);

			fireEvent.click(await screen.findByText("Explored /src/persist.ts"));
			expect(screen.getByText("persistent file contents")).toBeInTheDocument();

			unmount();
			render(<ToolActivity entry={entry} result={result} index={0} />);

			expect(screen.getByText("persistent file contents")).toBeInTheDocument();
		});
	});

	describe("basePath shortening", () => {
		it("shortens file_path label when basePath is provided", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/home/user/project/src/main.ts" },
				id: "t-bp1",
			};
			render(
				<ToolActivity entry={entry} index={0} basePath="/home/user/project" />,
			);

			await waitFor(() =>
				expect(screen.getByTestId("activity-tool-use-0")).toHaveTextContent(
					"Explored src/main.ts",
				),
			);
		});

		it("shortens file_path in default tool when basePath is provided", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/home/user/project/src/app.ts" },
				id: "t-bp2",
			};
			render(
				<ToolActivity entry={entry} index={0} basePath="/home/user/project" />,
			);

			await waitFor(() =>
				expect(screen.getByTestId("activity-tool-use-0")).toHaveTextContent(
					"Edit src/app.ts",
				),
			);
		});
	});

	describe("truncate class", () => {
		it("applies truncate to read tool label", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t-tr1",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const el = screen.getByTestId("activity-tool-use-0");
			const span = el.querySelector("span.truncate");
			expect(span).not.toBeNull();
		});

		it("applies truncate to command tool label", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "git status" },
				id: "t-tr2",
			};
			render(<ToolActivity entry={entry} index={0} />);

			await waitFor(() => {
				const el = screen.getByTestId("activity-tool-use-0");
				const span = el.querySelector("span.truncate");
				expect(span).not.toBeNull();
			});
		});

		it("applies truncate to default tool label", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts" },
				id: "t-tr3",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const el = screen.getByTestId("activity-tool-use-0");
			const span = el.querySelector("span.truncate");
			expect(span).not.toBeNull();
		});
	});

	describe("CommandToolActivity", () => {
		it("shows command with terminal icon", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "git status" },
				id: "t3",
			};
			render(<ToolActivity entry={entry} index={0} />);

			await waitFor(() =>
				expect(screen.getByTestId("activity-tool-use-0")).toHaveTextContent(
					"git status",
				),
			);
		});

		it("does not render completed text in the command header", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "git status" },
				id: "t3-done",
			};
			const result = {
				type: "tool_result" as const,
				content: "clean",
				isError: false,
			};
			render(<ToolActivity entry={entry} result={result} index={0} />);

			const header = await screen.findByTestId("activity-tool-use-0");
			expect(header).toHaveTextContent("git status");
			expect(header).not.toHaveTextContent("completed");
		});

		it("shows result as collapsible on label click", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "ls" },
				id: "t4",
			};
			const result = {
				type: "tool_result" as const,
				content: "file1.ts\nfile2.ts",
				isError: false,
			};
			render(<ToolActivity entry={entry} result={result} index={0} />);

			expect(screen.queryByText(/file1\.ts/)).toBeNull();

			fireEvent.click(await screen.findByText("ls"));
			expect(screen.getByText(/file1\.ts/)).toBeInTheDocument();
		});

		it("shows error result as collapsible", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "bad-cmd" },
				id: "t5",
			};
			const result = {
				type: "tool_result" as const,
				content: "command not found",
				isError: true,
			};
			render(<ToolActivity entry={entry} result={result} index={0} />);

			expect(screen.getByText("Error")).toBeInTheDocument();
			expect(screen.queryByText("command not found")).toBeNull();

			fireEvent.click(screen.getByText("Error"));
			expect(screen.getByText("command not found")).toBeInTheDocument();
		});
	});

	describe("isExecuting spinner", () => {
		it("shows spinner when isExecuting is true for read tool", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} isExecuting={true} />);
			const el = screen.getByTestId("activity-tool-use-0");
			expect(el.querySelector(".animate-spin")).not.toBeNull();
		});

		it("shows chevron when isExecuting is false for read tool", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t1",
			};
			const result = {
				type: "tool_result" as const,
				content: "contents",
				isError: false,
			};
			render(
				<ToolActivity
					entry={entry}
					result={result}
					index={0}
					isExecuting={false}
				/>,
			);
			const el = screen.getByTestId("activity-tool-use-0");
			expect(el.querySelector(".animate-spin")).toBeNull();
		});

		it("shows spinner when isExecuting is true for command tool", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "git status" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} isExecuting={true} />);
			const el = screen.getByTestId("activity-tool-use-0");
			expect(el.querySelector(".animate-spin")).not.toBeNull();
		});

		it("does not auto-expand command tools while executing", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "git status" },
				id: "t-running-command-collapsed",
			};
			render(<ToolActivity entry={entry} index={0} isExecuting={true} />);

			expect(screen.queryByLabelText("Copy command")).not.toBeInTheDocument();
		});

		it("shows spinner when isExecuting is true for default tool", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} isExecuting={true} />);
			const el = screen.getByTestId("activity-tool-use-0");
			expect(el.querySelector(".animate-spin")).not.toBeNull();
		});

		it("allows expand while executing", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} isExecuting={true} />);
			fireEvent.click(screen.getByText(/Edit/));
			expect(
				screen.getByText(/"file_path": "\/src\/app.ts"/),
			).toBeInTheDocument();
		});
	});

	describe("DefaultToolActivity", () => {
		it("shows write tool with name and summary", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts", old_string: "a", new_string: "b" },
				id: "t6",
			};
			render(<ToolActivity entry={entry} index={0} />);

			await waitFor(() => {
				const el = screen.getByTestId("activity-tool-use-0");
				expect(el).toHaveTextContent("Edit");
				expect(el).toHaveTextContent("/src/app.ts");
			});
		});

		it("toggles expand/collapse on label click", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts" },
				id: "t7",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const label = await screen.findByText("Edit /src/app.ts");
			fireEvent.click(label);
			expect(
				screen.getByText(/"file_path": "\/src\/app.ts"/),
			).toBeInTheDocument();

			fireEvent.click(label);
			expect(screen.queryByText(/"file_path": "\/src\/app.ts"/)).toBeNull();
		});

		it("shows Rust-built inline diff preview for edit tools", async () => {
			mockInvoke.mockImplementation((command: string, args: unknown) => {
				if (command === "present_agent_tool_activity") {
					return Promise.resolve(
						presentToolActivity(
							args as {
								toolName: string;
								input: Record<string, unknown>;
								basePath?: string;
							},
						),
					);
				}
				if (command === "get_language_from_path") {
					return Promise.resolve("typescript");
				}
				if (command === "compute_diff_hunks") {
					return Promise.resolve({
						hunks: [
							{
								index: 0,
								oldStart: 1,
								oldLines: 1,
								newStart: 1,
								newLines: 1,
								lines: ["-old", "+new"],
							},
						],
					});
				}
				if (command !== "build_agent_edit_preview") {
					return Promise.resolve(null);
				}
				return Promise.resolve({
					toolName: "Edit",
					operation: "Edit first match",
					filePath: "src/app.ts",
					originalContent: "old",
					modifiedContent: "new",
					hunks: [
						{
							oldStart: 1,
							newStart: 1,
							lines: [
								{
									kind: "removed",
									oldLine: 1,
									newLine: null,
									content: "old",
								},
								{
									kind: "added",
									oldLine: null,
									newLine: 1,
									content: "new",
								},
							],
						},
					],
					warnings: [],
				});
			});
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: {
					file_path: "src/app.ts",
					old_string: "old",
					new_string: "new",
				},
				id: "t8",
			};
			render(<ToolActivity entry={entry} index={0} basePath="/repo" />);

			fireEvent.click(await screen.findByText("Edit src/app.ts"));

			await waitFor(() =>
				expect(mockInvoke).toHaveBeenCalledWith("build_agent_edit_preview", {
					worktreePath: "/repo",
					toolName: "Edit",
					input: entry.input,
				}),
			);
			expect(
				await screen.findByText("Edit first match: src/app.ts"),
			).toBeInTheDocument();
			expect(screen.getByText("old")).toBeInTheDocument();
			expect(screen.getByText("new")).toBeInTheDocument();
		});

		it("shows an edit diff loading row while the preview is being built", async () => {
			let resolvePreview: (value: unknown) => void = () => {};
			mockInvoke.mockImplementation((command: string, args: unknown) => {
				if (command === "present_agent_tool_activity") {
					return Promise.resolve(
						presentToolActivity(
							args as {
								toolName: string;
								input: Record<string, unknown>;
								basePath?: string;
							},
						),
					);
				}
				if (command === "build_agent_edit_preview") {
					return new Promise((resolve) => {
						resolvePreview = resolve;
					});
				}
				return Promise.resolve(null);
			});
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: {
					file_path: "src/loading.ts",
					old_string: "old",
					new_string: "new",
				},
				id: "t-edit-preview-loading",
			};
			render(<ToolActivity entry={entry} index={0} basePath="/repo" />);

			fireEvent.click(await screen.findByText("Edit src/loading.ts"));
			expect(screen.getByText("Loading edit diff...")).toBeInTheDocument();

			resolvePreview({
				toolName: "Edit",
				operation: "Edit first match",
				filePath: "src/loading.ts",
				originalContent: "old",
				modifiedContent: "new",
				hunks: [
					{
						oldStart: 1,
						newStart: 1,
						lines: [
							{
								kind: "removed",
								oldLine: 1,
								newLine: null,
								content: "old",
							},
							{
								kind: "added",
								oldLine: null,
								newLine: 1,
								content: "new",
							},
						],
					},
				],
				warnings: [],
			});

			expect(
				await screen.findByText("Edit first match: src/loading.ts"),
			).toBeInTheDocument();
		});

		it("does not rebuild edit preview when semantic input is unchanged", async () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: {
					file_path: "src/stable.ts",
					old_string: "old",
					new_string: "new",
				},
				id: "t-edit-preview-stable",
			};
			const { rerender } = render(
				<ToolActivity entry={entry} index={0} basePath="/repo" />,
			);

			fireEvent.click(await screen.findByText("Edit src/stable.ts"));
			await screen.findByText("Edit first match: src/stable.ts");
			const previewCalls = mockInvoke.mock.calls.filter(
				([command]) => command === "build_agent_edit_preview",
			);
			expect(previewCalls).toHaveLength(1);

			rerender(
				<ToolActivity
					entry={{
						...entry,
						input: {
							file_path: "src/stable.ts",
							old_string: "old",
							new_string: "new",
						},
					}}
					index={0}
					basePath="/repo"
				/>,
			);

			expect(
				screen.getByText("Edit first match: src/stable.ts"),
			).toBeInTheDocument();
			const nextPreviewCalls = mockInvoke.mock.calls.filter(
				([command]) => command === "build_agent_edit_preview",
			);
			expect(nextPreviewCalls).toHaveLength(1);
		});
	});
});

describe("ActivityItem", () => {
	it("shows tool_result with error as collapsible", () => {
		const entry: ActivityEntry = {
			type: "tool_result",
			content: "permission denied",
			isError: true,
		};
		render(<ActivityItem entry={entry} index={1} />);

		const el = screen.getByTestId("activity-tool-result-1");
		expect(el).toHaveTextContent("Error");
		expect(screen.queryByText("permission denied")).toBeNull();

		fireEvent.click(screen.getByText("Error"));
		expect(screen.getByText("permission denied")).toBeInTheDocument();
	});

	it("shows tool_result without error", () => {
		const entry: ActivityEntry = {
			type: "tool_result",
			content: "file contents",
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);

		const el = screen.getByTestId("activity-tool-result-1");
		expect(el).toHaveTextContent("✓");
	});

	it("shows 'Done' for empty content result", () => {
		const entry: ActivityEntry = {
			type: "tool_result",
			content: "",
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);

		const el = screen.getByTestId("activity-tool-result-1");
		expect(el).toHaveTextContent("Done");
	});

	it("toggles expand/collapse for result content", () => {
		const entry: ActivityEntry = {
			type: "tool_result",
			content: "output text",
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);

		const label = screen.getByText("✓");
		fireEvent.click(label);

		expect(screen.getByText("output text")).toBeInTheDocument();

		fireEvent.click(label);
		expect(screen.queryByText("output text")).toBeNull();
	});

	it("truncates long result content", () => {
		const longContent = Array.from(
			{ length: 20 },
			(_, i) => `line ${i + 1}`,
		).join("\n");
		const entry: ActivityEntry = {
			type: "tool_result",
			content: longContent,
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);
		fireEvent.click(screen.getByText("✓"));

		expect(screen.getByText(/15 more lines/)).toBeInTheDocument();
	});

	it("truncates single-line huge result content by characters", () => {
		const longContent = "x".repeat(5000);
		const entry: ActivityEntry = {
			type: "tool_result",
			content: longContent,
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);
		fireEvent.click(screen.getByText("✓"));

		expect(screen.getByText(/more chars/)).toBeInTheDocument();
		expect(screen.queryByText(longContent)).not.toBeInTheDocument();
	});

	it("copies full tool result content", async () => {
		const content = Array.from({ length: 8 }, (_, i) => `line ${i + 1}`).join(
			"\n",
		);
		const entry: ActivityEntry = {
			type: "tool_result",
			content,
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);

		fireEvent.click(screen.getByText("✓"));
		fireEvent.click(screen.getByLabelText("Copy tool result"));

		await waitFor(() =>
			expect(clipboardWriteText).toHaveBeenCalledWith(content),
		);
	});

	it("shows tool_use fallback when no paired result", () => {
		const entry: ActivityEntry = {
			type: "tool_use",
			tool: "Read",
			input: { file_path: "/src/main.ts" },
			id: "t-orphan",
		};
		render(<ActivityItem entry={entry} index={2} />);
		expect(screen.getByTestId("activity-tool-use-2")).toBeDefined();
	});

	it("shows permission_result with allowed status", () => {
		const entry: ActivityEntry = {
			type: "permission_result",
			toolName: "Bash",
			status: "allowed",
			summary: "Bash: allowed",
		};
		render(<ActivityItem entry={entry} index={0} />);

		const el = screen.getByTestId("activity-permission-result-0");
		expect(el).toHaveTextContent("✓");
		expect(el).toHaveTextContent("Bash");
	});
});

describe("TaskToolActivity", () => {
	function makeTaskGroup(overrides: Partial<TaskGroup> = {}): TaskGroup {
		return {
			toolUseIndex: 0,
			toolUseId: "toolu_task_001",
			description: "Explore codebase",
			subagentType: "Explore",
			childIndices: [],
			statusParts: [],
			resultIndex: undefined,
			isCompleted: false,
			isBackground: false,
			completionStatusIndex: undefined,
			...overrides,
		};
	}

	const baseParts: MessagePart[] = [
		{
			type: "tool_use",
			tool: "Task",
			input: { description: "Explore codebase", subagent_type: "Explore" },
			id: "toolu_task_001",
		},
	];

	it("shows completed task in collapsed state with description and subagentType", () => {
		const group = makeTaskGroup({
			isCompleted: true,
			childIndices: [1, 2],
			statusParts: [
				{
					type: "task_status",
					taskToolUseId: "toolu_task_001",
					status: "completed",
					summary: "Done",
				},
			],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
			{
				type: "tool_result",
				content: "file content",
				isError: false,
				toolUseId: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		expect(el).toHaveTextContent("Explore codebase");
		// Children should not be visible (collapsed)
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();
	});

	it("shows running task in collapsed state by default", () => {
		const group = makeTaskGroup({
			childIndices: [1],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={true}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		// Running task shows spinner
		expect(el.querySelector(".animate-spin")).not.toBeNull();
		// Children not visible (collapsed by default)
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();
	});

	it("shows spinner for background task even when not streaming", () => {
		const group = makeTaskGroup({
			isBackground: true,
			isCompleted: false,
			childIndices: [1],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		expect(el.querySelector(".animate-spin")).not.toBeNull();
	});

	it("hides spinner for completed background task", () => {
		const group = makeTaskGroup({
			isBackground: true,
			isCompleted: true,
			childIndices: [1],
			statusParts: [
				{
					type: "task_status",
					taskToolUseId: "toolu_task_001",
					status: "completed",
					summary: "Done",
				},
			],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		expect(el.querySelector(".animate-spin")).toBeNull();
	});

	it("toggles expand/collapse on label click", () => {
		const group = makeTaskGroup({
			isCompleted: true,
			childIndices: [1],
			statusParts: [
				{
					type: "task_status",
					taskToolUseId: "toolu_task_001",
					status: "completed",
				},
			],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		// Completed → initially collapsed
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();

		// Click to expand
		fireEvent.click(screen.getByTestId("activity-task-0"));
		expect(screen.getByTestId("activity-tool-use-1")).toBeDefined();

		// Click to collapse
		fireEvent.click(screen.getByTestId("activity-task-0"));
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();
	});

	it("keeps expanded task content open after remounting the same task", async () => {
		const group = makeTaskGroup({
			toolUseId: "toolu_task_persist",
			childIndices: [1],
		});
		const parts: MessagePart[] = [
			{
				type: "tool_use",
				tool: "Task",
				input: {
					description: "Persist task",
					subagent_type: "Explore",
				},
				id: "toolu_task_persist",
			},
			{
				type: "text",
				content: "persisted child output",
				parentToolUseId: "toolu_task_persist",
			},
		];
		const { unmount } = render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		fireEvent.click(screen.getByTestId("activity-task-0"));
		expect(screen.getByText("persisted child output")).toBeInTheDocument();

		unmount();
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		expect(screen.getByText("persisted child output")).toBeInTheDocument();
	});

	it("shows child tool_use entries when expanded by click", async () => {
		const childResult: Extract<MessagePart, { type: "tool_result" }> = {
			type: "tool_result",
			content: "file content here",
			isError: false,
			toolUseId: "toolu_child_001",
		};
		const group = makeTaskGroup({
			childIndices: [1],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		const pairedResults = new Map<
			number,
			Extract<MessagePart, { type: "tool_result" }>
		>([[1, childResult]]);

		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={pairedResults}
				isStreaming={false}
			/>,
		);

		// Collapsed by default
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();

		// Click to expand
		fireEvent.click(screen.getByTestId("activity-task-0"));
		await waitFor(() =>
			expect(screen.getByTestId("activity-tool-use-1")).toHaveTextContent(
				"Explored /src/main.ts",
			),
		);
	});

	it("shows description with subagentType suffix when both present", () => {
		const group = makeTaskGroup({
			isCompleted: true,
			childIndices: [],
			statusParts: [
				{
					type: "task_status",
					taskToolUseId: "toolu_task_001",
					status: "completed",
				},
			],
		});
		render(
			<TaskToolActivity
				group={group}
				parts={baseParts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		expect(el).toHaveTextContent("Explore codebase (Explore)");
	});

	it("shows sub-agent text output when expanded by click", () => {
		const group = makeTaskGroup({
			childIndices: [1],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "text",
				content: "Analysis result: found 3 components",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		// Collapsed by default
		expect(
			screen.queryByText("Analysis result: found 3 components"),
		).toBeNull();

		// Click to expand
		fireEvent.click(screen.getByTestId("activity-task-0"));
		expect(
			screen.getByText("Analysis result: found 3 components"),
		).toBeInTheDocument();
	});

	it("shows fallback label with subagentType when no description", () => {
		const group = makeTaskGroup({
			description: undefined,
			subagentType: "Explore",
		});
		render(
			<TaskToolActivity
				group={group}
				parts={baseParts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		expect(screen.getByTestId("activity-task-0")).toHaveTextContent(
			"Task (Explore)",
		);
	});
});
