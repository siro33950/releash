import {
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PermissionDialog } from "./PermissionDialog";

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

function resolvedInvoke<T>(value: T): Promise<T> {
	return {
		// biome-ignore lint/suspicious/noThenProperty: Tauri invoke is mocked as a synchronously settled thenable so effect-driven presentation is available during render tests.
		then: (
			onFulfilled?: (value: T) => unknown,
			onRejected?: (reason: unknown) => unknown,
		) => {
			try {
				const nextValue = onFulfilled ? onFulfilled(value) : value;
				return { catch: () => nextValue };
			} catch (error) {
				return { catch: () => onRejected?.(error) };
			}
		},
		catch: () => resolvedInvoke(value),
	} as unknown as Promise<T>;
}

function buildPermissionPresentation({
	toolName,
	input,
}: {
	toolName: string;
	input?: Record<string, unknown>;
}) {
	const source = input ?? {};
	const edits = Array.isArray(source.edits)
		? (source.edits as Record<string, unknown>[])
		: [];
	const allowedPrompts = Array.isArray(source.allowedPrompts)
		? source.allowedPrompts
		: [];
	const questions = Array.isArray(source.questions) ? source.questions : [];
	const kind =
		toolName === "ExitPlanMode"
			? "exit_plan"
			: toolName === "AskUserQuestion"
				? "ask_user_question"
				: "tool";
	const directContentKey = toolName === "Write" ? "content" : "new_string";
	return {
		kind,
		canEditInput: ["Edit", "MultiEdit", "Write"].includes(toolName),
		canEditContent: ["Edit", "Write"].includes(toolName),
		canEditMultiEditContent: toolName === "MultiEdit",
		directContentEditLabel:
			toolName === "Write"
				? "Edit file content"
				: toolName === "Edit"
					? "Edit replacement content"
					: null,
		directContent:
			typeof source[directContentKey] === "string"
				? source[directContentKey]
				: "",
		multiEditReplacementContents: edits.map((edit) =>
			typeof edit.new_string === "string" ? edit.new_string : "",
		),
		multiEditOldStrings: edits.map((edit) =>
			typeof edit.old_string === "string" ? edit.old_string : "",
		),
		hasResolvedDetail:
			kind === "exit_plan"
				? Boolean(source.plan) || allowedPrompts.length > 0
				: kind === "ask_user_question"
					? questions.length > 0
					: Object.keys(source).length > 0,
		plan: typeof source.plan === "string" ? source.plan : "",
		allowedPrompts,
		questions,
	};
}

const permissionPresentationById = new Map<
	string,
	ReturnType<typeof buildPermissionPresentation>
>();

function setPermissionPresentation(request: {
	id: string;
	toolName: string;
	input?: Record<string, unknown>;
}) {
	permissionPresentationById.set(
		request.id,
		buildPermissionPresentation(request),
	);
}

function fallbackPermissionPresentation(requestId: string) {
	const exitPlanInputs: Record<string, Record<string, unknown>> = {
		"req-exitplan-001": {
			plan: "# My Plan\n\n- Step 1\n- Step 2",
			planFilePath: "/tmp/plan.md",
		},
		"req-exitplan-002": {
			plan: "Some plan",
			planFilePath: "/tmp/secret/plan.md",
		},
		"req-exitplan-003": {
			plan: "Plan text",
			allowedPrompts: [
				{ tool: "Bash", prompt: "run tests" },
				{ tool: "Bash", prompt: "install dependencies" },
			],
		},
		"req-exitplan-004": { plan: "Plan only" },
		"req-exitplan-005": { plan: "Plan" },
		"req-exitplan-006": { plan: "Plan" },
	};
	if (requestId in exitPlanInputs) {
		return buildPermissionPresentation({
			toolName: "ExitPlanMode",
			input: exitPlanInputs[requestId],
		});
	}
	if (requestId === "req-ask-001") {
		return buildPermissionPresentation({
			toolName: "AskUserQuestion",
			input: {
				questions: [
					{
						question: "Which library should we use?",
						header: "Library",
						options: [
							{ label: "React", description: "Popular UI framework" },
							{ label: "Vue", description: "Progressive framework" },
						],
						multiSelect: false,
					},
				],
			},
		});
	}
	if (requestId === "req-ask-multi-001") {
		return buildPermissionPresentation({
			toolName: "AskUserQuestion",
			input: {
				questions: [
					{
						question: "Which features do you want?",
						header: "Features",
						options: [
							{ label: "Auth", description: "Authentication" },
							{ label: "DB", description: "Database" },
							{ label: "API", description: "REST API" },
						],
						multiSelect: true,
					},
				],
			},
		});
	}
	if (requestId === "req-ask-md-001") {
		return buildPermissionPresentation({
			toolName: "AskUserQuestion",
			input: {
				questions: [
					{
						question: "Use `react-markdown` for rendering?",
						header: "Choose a `markdown` library",
						options: [
							{
								label: "Yes",
								description: "Uses `react-markdown` with **remark-gfm**",
							},
							{ label: "No", description: "Plain text only" },
						],
						multiSelect: false,
					},
				],
			},
		});
	}
	return null;
}

function mockPermissionPresentation(command: string, args: unknown) {
	if (command === "present_agent_permission_request") {
		const { requestId } = args as { requestId?: string };
		return resolvedInvoke(
			requestId
				? (permissionPresentationById.get(requestId) ??
						fallbackPermissionPresentation(requestId))
				: null,
		);
	}
	if (command === "get_language_from_path") {
		return resolvedInvoke("typescript");
	}
	return null;
}

beforeEach(() => {
	permissionPresentationById.clear();
	setPermissionPresentation(baseRequest);
	mockInvoke.mockClear();
	mockInvoke.mockImplementation(
		(command: string, args: unknown) =>
			mockPermissionPresentation(command, args) ?? resolvedInvoke(null),
	);
});

const baseRequest = {
	id: "req-001",
	toolName: "Edit",
	input: { file_path: "/src/index.ts" },
	toolUseId: "toolu_001",
};

describe("PermissionDialog", () => {
	it("displays tool name when no title or displayName", () => {
		render(
			<PermissionDialog
				request={baseRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		expect(screen.getByText("Permission required: Edit")).toBeInTheDocument();
	});

	it("loads presentation by session and request id without sending request payload", () => {
		render(
			<PermissionDialog
				request={baseRequest}
				sessionId="session-1"
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);

		expect(mockInvoke).toHaveBeenCalledWith(
			"present_agent_permission_request",
			{
				chatSessionId: "session-1",
				requestId: "req-001",
			},
		);
		expect(
			mockInvoke.mock.calls
				.filter(([command]) => command === "present_agent_permission_request")
				.some(([, args]) =>
					Object.keys(args as Record<string, unknown>).includes("request"),
				),
		).toBe(false);
	});

	it("displays title when provided", () => {
		render(
			<PermissionDialog
				request={{ ...baseRequest, title: "Edit file" }}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		expect(
			screen.getByText("Permission required: Edit file"),
		).toBeInTheDocument();
	});

	it("displays description when provided", () => {
		render(
			<PermissionDialog
				request={{
					...baseRequest,
					description: "Modify /src/index.ts",
				}}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		expect(screen.getByText("Modify /src/index.ts")).toBeInTheDocument();
	});

	it("displays input as formatted JSON", () => {
		render(
			<PermissionDialog
				request={{
					...baseRequest,
					input: { file_path: "/src/index.ts", content: "hello" },
				}}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		const pre = screen.getByTestId("permission-input");
		expect(pre).toBeInTheDocument();
		const parsed = JSON.parse(pre.textContent ?? "");
		expect(parsed).toEqual({
			file_path: "/src/index.ts",
			content: "hello",
		});
	});

	it("shows Rust-built inline diff preview for edit permission requests", async () => {
		mockInvoke.mockImplementation((command: string, args: unknown) => {
			const presentation = mockPermissionPresentation(command, args);
			if (presentation) return presentation;
			if (command === "build_agent_edited_tool_input") {
				const input = (args as { input: Record<string, unknown> }).input;
				return Promise.resolve(input);
			}
			if (command === "build_agent_edit_preview") {
				return Promise.resolve({
					toolName: "Edit",
					operation: "Edit first match",
					filePath: "src/index.ts",
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
								{ kind: "added", oldLine: null, newLine: 1, content: "new" },
							],
						},
					],
					warnings: [],
				});
			}
			return Promise.resolve(null);
		});
		const request = {
			...baseRequest,
			input: {
				file_path: "src/index.ts",
				old_string: "old",
				new_string: "new",
			},
		};

		render(
			<PermissionDialog
				request={request}
				worktreePath="/repo"
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);

		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith("build_agent_edit_preview", {
				worktreePath: "/repo",
				toolName: "Edit",
				input: request.input,
			}),
		);
		expect(
			await screen.findByText("Edit first match: src/index.ts"),
		).toBeInTheDocument();
		expect(screen.getByText("old")).toBeInTheDocument();
		expect(screen.getAllByText("new").length).toBeGreaterThan(0);
	});

	it("hides input section when input is empty", () => {
		render(
			<PermissionDialog
				request={{ ...baseRequest, input: {} }}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		expect(screen.queryByTestId("permission-input")).not.toBeInTheDocument();
	});

	it("calls onAllow with request_id when Allow is clicked", async () => {
		const onAllow = vi.fn();
		render(
			<PermissionDialog
				request={baseRequest}
				onAllow={onAllow}
				onDeny={vi.fn()}
			/>,
		);

		await userEvent.click(screen.getByText("Allow"));
		expect(onAllow).toHaveBeenCalledWith("req-001");
	});

	it("allows an edited tool input for edit permission requests", async () => {
		const onAllow = vi.fn();
		render(
			<PermissionDialog
				request={{
					...baseRequest,
					input: {
						file_path: "src/index.ts",
						old_string: "old",
						new_string: "new",
					},
				}}
				onAllow={onAllow}
				onDeny={vi.fn()}
			/>,
		);
		const editor = screen.getByLabelText("Edit permission input JSON");

		fireEvent.change(editor, {
			target: {
				value: JSON.stringify({
					file_path: "src/index.ts",
					old_string: "old",
					new_string: "edited",
				}),
			},
		});
		await userEvent.click(screen.getByText("Allow edited"));

		expect(onAllow).toHaveBeenCalledWith("req-001", {
			file_path: "src/index.ts",
			old_string: "old",
			new_string: "edited",
		});
	});

	it("allows direct replacement content edits through Rust", async () => {
		const onAllow = vi.fn();
		mockInvoke.mockImplementation((command: string, args: unknown) => {
			const presentation = mockPermissionPresentation(command, args);
			if (presentation) return presentation;
			if (command === "build_agent_edited_tool_input") {
				return Promise.resolve({
					file_path: "src/index.ts",
					old_string: "old",
					new_string: "direct edit",
				});
			}
			return Promise.resolve(null);
		});
		render(
			<PermissionDialog
				request={{
					...baseRequest,
					input: {
						file_path: "src/index.ts",
						old_string: "old",
						new_string: "new",
					},
				}}
				onAllow={onAllow}
				onDeny={vi.fn()}
			/>,
		);
		const editor = screen.getByLabelText("Edit replacement content");

		fireEvent.change(editor, {
			target: {
				value: "direct edit",
			},
		});
		await userEvent.click(screen.getByText("Allow content edit"));

		expect(mockInvoke).toHaveBeenCalledWith("build_agent_edited_tool_input", {
			toolName: "Edit",
			input: {
				file_path: "src/index.ts",
				old_string: "old",
				new_string: "new",
			},
			editedContent: "direct edit",
		});
		expect(onAllow).toHaveBeenCalledWith("req-001", {
			file_path: "src/index.ts",
			old_string: "old",
			new_string: "direct edit",
		});
	});

	it("updates inline diff preview after direct replacement content edits", async () => {
		mockInvoke.mockImplementation(
			(command: string, args: Record<string, unknown>) => {
				const presentation = mockPermissionPresentation(command, args);
				if (presentation) return presentation;
				if (command === "build_agent_edited_tool_input") {
					return Promise.resolve({
						file_path: "src/index.ts",
						old_string: "old",
						new_string: args.editedContent,
					});
				}
				if (command === "build_agent_edit_preview") {
					const input = args.input as Record<string, unknown>;
					return Promise.resolve({
						toolName: "Edit",
						operation: "Edit first match",
						filePath: "src/index.ts",
						originalContent: "old",
						modifiedContent: String(input.new_string),
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
										content: String(input.new_string),
									},
								],
							},
						],
						warnings: [],
					});
				}
				return Promise.resolve(null);
			},
		);
		render(
			<PermissionDialog
				request={{
					...baseRequest,
					input: {
						file_path: "src/index.ts",
						old_string: "old",
						new_string: "new",
					},
				}}
				worktreePath="/repo"
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);

		fireEvent.change(screen.getByLabelText("Edit replacement content"), {
			target: {
				value: "preview edit",
			},
		});

		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith("build_agent_edit_preview", {
				worktreePath: "/repo",
				toolName: "Edit",
				input: {
					file_path: "src/index.ts",
					old_string: "old",
					new_string: "preview edit",
				},
			}),
		);
		await screen.findByText("Edit first match: src/index.ts");
		await waitFor(() =>
			expect(screen.getAllByText("preview edit").length).toBeGreaterThan(1),
		);
	});

	it("allows direct MultiEdit replacement content edits through Rust", async () => {
		const onAllow = vi.fn();
		mockInvoke.mockImplementation((command: string, args: unknown) => {
			const presentation = mockPermissionPresentation(command, args);
			if (presentation) return presentation;
			if (command === "build_agent_edited_multi_edit_tool_input") {
				return Promise.resolve({
					file_path: "src/index.ts",
					edits: [
						{ old_string: "one", new_string: "two" },
						{ old_string: "three", new_string: "direct multi edit" },
					],
				});
			}
			return Promise.resolve(null);
		});
		const input = {
			file_path: "src/index.ts",
			edits: [
				{ old_string: "one", new_string: "two" },
				{ old_string: "three", new_string: "four" },
			],
		};
		setPermissionPresentation({
			...baseRequest,
			toolName: "MultiEdit",
			input,
		});
		render(
			<PermissionDialog
				request={{
					...baseRequest,
					toolName: "MultiEdit",
					input,
				}}
				onAllow={onAllow}
				onDeny={vi.fn()}
			/>,
		);
		const editor = screen.getByLabelText("Edit replacement content 2");

		fireEvent.change(editor, {
			target: {
				value: "direct multi edit",
			},
		});
		await userEvent.click(screen.getByText("Allow edit 2"));

		expect(mockInvoke).toHaveBeenCalledWith(
			"build_agent_edited_multi_edit_tool_input",
			{
				input,
				editIndex: 1,
				editedContent: "direct multi edit",
			},
		);
		expect(onAllow).toHaveBeenCalledWith("req-001", {
			file_path: "src/index.ts",
			edits: [
				{ old_string: "one", new_string: "two" },
				{ old_string: "three", new_string: "direct multi edit" },
			],
		});
	});

	it("calls onDeny with request_id when Deny is clicked", async () => {
		const onDeny = vi.fn();
		render(
			<PermissionDialog
				request={baseRequest}
				onAllow={vi.fn()}
				onDeny={onDeny}
			/>,
		);

		await userEvent.click(screen.getByText("Deny"));
		expect(onDeny).toHaveBeenCalledWith("req-001");
	});
});

describe("PermissionDialog — ExitPlanMode", () => {
	it("renders plan as markdown", () => {
		render(
			<PermissionDialog
				request={{
					id: "req-exitplan-001",
					toolName: "ExitPlanMode",
					input: {
						plan: "# My Plan\n\n- Step 1\n- Step 2",
						planFilePath: "/tmp/plan.md",
					},
					toolUseId: "toolu_exit_001",
				}}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		const mdContainer = screen.getByTestId("plan-markdown");
		expect(mdContainer).toBeInTheDocument();
		expect(mdContainer.querySelector("h1")).toBeTruthy();
		expect(mdContainer.querySelectorAll("li")).toHaveLength(2);
	});

	it("does not display planFilePath", () => {
		render(
			<PermissionDialog
				request={{
					id: "req-exitplan-002",
					toolName: "ExitPlanMode",
					input: {
						plan: "Some plan",
						planFilePath: "/tmp/secret/plan.md",
					},
					toolUseId: "toolu_exit_002",
				}}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		expect(screen.queryByText("/tmp/secret/plan.md")).not.toBeInTheDocument();
	});

	it("displays allowedPrompts as a list", () => {
		render(
			<PermissionDialog
				request={{
					id: "req-exitplan-003",
					toolName: "ExitPlanMode",
					input: {
						plan: "Plan text",
						allowedPrompts: [
							{ tool: "Bash", prompt: "run tests" },
							{ tool: "Bash", prompt: "install dependencies" },
						],
					},
					toolUseId: "toolu_exit_003",
				}}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		const section = screen.getByTestId("allowed-prompts");
		expect(section).toBeInTheDocument();
		expect(screen.getByText("Bash: run tests")).toBeInTheDocument();
		expect(screen.getByText("Bash: install dependencies")).toBeInTheDocument();
	});

	it("hides allowedPrompts section when not provided", () => {
		render(
			<PermissionDialog
				request={{
					id: "req-exitplan-004",
					toolName: "ExitPlanMode",
					input: { plan: "Plan only" },
					toolUseId: "toolu_exit_004",
				}}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		expect(screen.queryByTestId("allowed-prompts")).not.toBeInTheDocument();
	});

	it("calls onAllow with request_id when Allow is clicked", async () => {
		const onAllow = vi.fn();
		render(
			<PermissionDialog
				request={{
					id: "req-exitplan-005",
					toolName: "ExitPlanMode",
					input: { plan: "Plan" },
					toolUseId: "toolu_exit_005",
				}}
				onAllow={onAllow}
				onDeny={vi.fn()}
			/>,
		);
		await userEvent.click(screen.getByText("Allow"));
		expect(onAllow).toHaveBeenCalledWith("req-exitplan-005");
		expect(onAllow).toHaveBeenCalledTimes(1);
	});

	it("calls onDeny with request_id when Deny is clicked", async () => {
		const onDeny = vi.fn();
		render(
			<PermissionDialog
				request={{
					id: "req-exitplan-006",
					toolName: "ExitPlanMode",
					input: { plan: "Plan" },
					toolUseId: "toolu_exit_006",
				}}
				onAllow={vi.fn()}
				onDeny={onDeny}
			/>,
		);
		await userEvent.click(screen.getByText("Deny"));
		expect(onDeny).toHaveBeenCalledWith("req-exitplan-006");
		expect(onDeny).toHaveBeenCalledTimes(1);
	});
});

const askRequest = {
	id: "req-ask-001",
	toolName: "AskUserQuestion",
	input: {
		questions: [
			{
				question: "Which library should we use?",
				header: "Library",
				options: [
					{ label: "React", description: "Popular UI framework" },
					{ label: "Vue", description: "Progressive framework" },
				],
				multiSelect: false,
			},
		],
	},
	toolUseId: "toolu_ask_001",
};

describe("PermissionDialog — AskUserQuestion", () => {
	it("displays question text and options as vertical list with radio buttons", () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		expect(
			screen.getByText("Which library should we use?"),
		).toBeInTheDocument();
		expect(screen.getByText("Library")).toBeInTheDocument();
		expect(screen.getByText("React")).toBeInTheDocument();
		expect(screen.getByText("Vue")).toBeInTheDocument();
		// Radio buttons are rendered for single-select
		const radios = screen.getAllByRole("radio");
		// 2 options + 1 Other = 3 radio items
		expect(radios).toHaveLength(3);
	});

	it("displays each option with label and description as a pair", () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		expect(screen.getByText("Popular UI framework")).toBeInTheDocument();
		expect(screen.getByText("Progressive framework")).toBeInTheDocument();
	});

	it("hides Allow/Deny buttons for AskUserQuestion", () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		expect(screen.queryByText("Allow")).not.toBeInTheDocument();
		expect(screen.queryByText("Deny")).not.toBeInTheDocument();
	});

	it("Submit is disabled until all questions are answered", () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		expect(screen.getByText("Submit")).toBeDisabled();
	});

	it("calls onAnswer with answers when option is selected and Submit clicked", async () => {
		const onAnswer = vi.fn();
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={onAnswer}
			/>,
		);

		await userEvent.click(screen.getByText("React"));
		expect(screen.getByText("Submit")).not.toBeDisabled();

		await userEvent.click(screen.getByText("Submit"));
		expect(onAnswer).toHaveBeenCalledWith("req-ask-001", {
			"Which library should we use?": "React",
		});
	});

	it("visually distinguishes the selected option (single select)", async () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);

		const radios = screen.getAllByRole("radio");
		// Initially no radio is checked
		for (const radio of radios) {
			expect(radio).not.toBeChecked();
		}

		await userEvent.click(screen.getByText("React"));
		// The first radio (React) should now be checked
		expect(radios[0]).toBeChecked();
		expect(radios[1]).not.toBeChecked();
	});

	it("handles multiple questions", async () => {
		const multiRequest = {
			...askRequest,
			input: {
				questions: [
					{
						question: "Pick a framework",
						header: "Framework",
						options: [
							{ label: "Next.js", description: "React meta-framework" },
							{ label: "Remix", description: "Full stack framework" },
						],
						multiSelect: false,
					},
					{
						question: "Pick a language",
						header: "Language",
						options: [
							{ label: "TypeScript", description: "Typed JS" },
							{ label: "JavaScript", description: "Plain JS" },
						],
						multiSelect: false,
					},
				],
			},
		};
		const onAnswer = vi.fn();
		setPermissionPresentation(multiRequest);
		render(
			<PermissionDialog
				request={multiRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={onAnswer}
			/>,
		);

		expect(screen.getByText("Submit")).toBeDisabled();

		await userEvent.click(screen.getByText("Next.js"));
		expect(screen.getByText("Submit")).toBeDisabled();

		await userEvent.click(screen.getByText("TypeScript"));
		expect(screen.getByText("Submit")).not.toBeDisabled();

		await userEvent.click(screen.getByText("Submit"));
		expect(onAnswer).toHaveBeenCalledWith("req-ask-001", {
			"Pick a framework": "Next.js",
			"Pick a language": "TypeScript",
		});
	});

	it("falls back to regular dialog when onAnswer is not provided", () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		expect(screen.getByText("Allow")).toBeInTheDocument();
		expect(screen.getByText("Deny")).toBeInTheDocument();
	});

	it("displays Other option in the same radio list", () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		expect(screen.getByText("Other")).toBeInTheDocument();
		// Other is a radio item in the same group
		const radios = screen.getAllByRole("radio");
		expect(radios).toHaveLength(3);
	});

	it("shows text input when Other is clicked", async () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);

		await userEvent.click(screen.getByText("Other"));
		expect(
			screen.getByLabelText("Other input for Which library should we use?"),
		).toBeInTheDocument();
	});

	it("keeps Submit disabled when Other is selected but text is empty", async () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);

		await userEvent.click(screen.getByText("Other"));
		expect(screen.getByText("Submit")).toBeDisabled();
	});

	it("submits other text as the answer value", async () => {
		const onAnswer = vi.fn();
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={onAnswer}
			/>,
		);

		await userEvent.click(screen.getByText("Other"));
		await userEvent.type(
			screen.getByLabelText("Other input for Which library should we use?"),
			"Svelte",
		);
		expect(screen.getByText("Submit")).not.toBeDisabled();

		await userEvent.click(screen.getByText("Submit"));
		expect(onAnswer).toHaveBeenCalledWith("req-ask-001", {
			"Which library should we use?": "Svelte",
		});
	});

	it("hides text input when a defined option is re-selected after Other", async () => {
		render(
			<PermissionDialog
				request={askRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);

		await userEvent.click(screen.getByText("Other"));
		expect(
			screen.getByLabelText("Other input for Which library should we use?"),
		).toBeInTheDocument();

		await userEvent.click(screen.getByText("React"));
		expect(
			screen.queryByLabelText("Other input for Which library should we use?"),
		).not.toBeInTheDocument();
	});

	it("shows proposed options with the selected one highlighted after answering", async () => {
		render(
			<PermissionDialog
				request={askRequest}
				status="allowed"
				resolvedAnswers={{ "Which library should we use?": "React" }}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		const resolved = screen.getByTestId("permission-resolved");
		// 「Choices」を展開すると提案された選択肢が表示される
		await userEvent.click(await within(resolved).findByText("Choices"));
		const options = within(resolved).getAllByTestId("resolved-option");
		expect(options).toHaveLength(2);
		expect(options.some((o) => o.textContent?.includes("React"))).toBe(true);
		expect(options.some((o) => o.textContent?.includes("Vue"))).toBe(true);
		// 選択した React だけがハイライト（data-selected=true）
		const selected = options.filter(
			(o) => o.getAttribute("data-selected") === "true",
		);
		expect(selected).toHaveLength(1);
		expect(selected[0].textContent).toContain("React");
	});

	it("highlights a single-select option whose label contains a comma", async () => {
		const commaRequest = {
			...askRequest,
			input: {
				questions: [
					{
						question: "Pick a stack",
						header: "Stack",
						options: [
							{ label: "React, Vite", description: "SPA" },
							{ label: "Next.js", description: "SSR" },
						],
						multiSelect: false,
					},
				],
			},
		};
		setPermissionPresentation(commaRequest);
		render(
			<PermissionDialog
				request={commaRequest}
				status="allowed"
				resolvedAnswers={{ "Pick a stack": "React, Vite" }}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		const resolved = screen.getByTestId("permission-resolved");
		await userEvent.click(await within(resolved).findByText("Choices"));
		const options = within(resolved).getAllByTestId("resolved-option");
		// 単一選択ではカンマを含むラベルが1要素として扱われ、正しくハイライトされる
		const selected = options.filter(
			(o) => o.getAttribute("data-selected") === "true",
		);
		expect(selected).toHaveLength(1);
		expect(selected[0].textContent).toContain("React, Vite");
	});
});

describe("AskUserQuestion — multiSelect", () => {
	const multiSelectRequest = {
		id: "req-ask-multi-001",
		toolName: "AskUserQuestion",
		input: {
			questions: [
				{
					question: "Which features do you want?",
					header: "Features",
					options: [
						{ label: "Auth", description: "Authentication" },
						{ label: "DB", description: "Database" },
						{ label: "API", description: "REST API" },
					],
					multiSelect: true,
				},
			],
		},
		toolUseId: "toolu_ask_multi_001",
	};

	it("renders checkboxes for multi-select questions", () => {
		render(
			<PermissionDialog
				request={multiSelectRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		const checkboxes = screen.getAllByRole("checkbox");
		expect(checkboxes).toHaveLength(3);
	});

	it("displays each option with label and description", () => {
		render(
			<PermissionDialog
				request={multiSelectRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		expect(screen.getByText("Auth")).toBeInTheDocument();
		expect(screen.getByText("Authentication")).toBeInTheDocument();
		expect(screen.getByText("DB")).toBeInTheDocument();
		expect(screen.getByText("Database")).toBeInTheDocument();
		expect(screen.getByText("API")).toBeInTheDocument();
		expect(screen.getByText("REST API")).toBeInTheDocument();
	});

	it("allows selecting multiple checkboxes and visually distinguishes them", async () => {
		render(
			<PermissionDialog
				request={multiSelectRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);

		const checkboxes = screen.getAllByRole("checkbox");

		await userEvent.click(screen.getByText("Auth"));
		await userEvent.click(screen.getByText("API"));

		expect(checkboxes[0]).toBeChecked();
		expect(checkboxes[1]).not.toBeChecked();
		expect(checkboxes[2]).toBeChecked();
	});

	it("submits comma-separated values for multi-select", async () => {
		const onAnswer = vi.fn();
		render(
			<PermissionDialog
				request={multiSelectRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={onAnswer}
			/>,
		);

		await userEvent.click(screen.getByText("Auth"));
		await userEvent.click(screen.getByText("DB"));
		expect(screen.getByText("Submit")).not.toBeDisabled();

		await userEvent.click(screen.getByText("Submit"));
		expect(onAnswer).toHaveBeenCalledWith("req-ask-multi-001", {
			"Which features do you want?": "Auth, DB",
		});
	});

	it("can toggle checkbox off", async () => {
		render(
			<PermissionDialog
				request={multiSelectRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);

		await userEvent.click(screen.getByText("Auth"));
		const checkboxes = screen.getAllByRole("checkbox");
		expect(checkboxes[0]).toBeChecked();

		await userEvent.click(screen.getByText("Auth"));
		expect(checkboxes[0]).not.toBeChecked();
	});

	it("does not show Other option for multi-select", () => {
		render(
			<PermissionDialog
				request={multiSelectRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		expect(screen.queryByText("Other")).not.toBeInTheDocument();
	});
});

describe("AskUserQuestion — markdown rendering", () => {
	const mdAskRequest = {
		id: "req-ask-md-001",
		toolName: "AskUserQuestion",
		input: {
			questions: [
				{
					question: "Use `react-markdown` for rendering?",
					header: "Choose a `markdown` library",
					options: [
						{
							label: "Yes",
							description: "Uses `react-markdown` with **remark-gfm**",
						},
						{ label: "No", description: "Plain text only" },
					],
					multiSelect: false,
				},
			],
		},
		toolUseId: "toolu_ask_md_001",
	};

	it("renders question text markdown as HTML", () => {
		render(
			<PermissionDialog
				request={mdAskRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		const dialog = screen.getByTestId("permission-dialog");
		expect(dialog.querySelector("code")).toBeTruthy();
		expect(dialog.textContent).toContain("react-markdown");
	});

	it("renders header markdown as HTML", () => {
		render(
			<PermissionDialog
				request={mdAskRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		const dialog = screen.getByTestId("permission-dialog");
		const codes = dialog.querySelectorAll("code");
		const codeTexts = Array.from(codes).map((c) => c.textContent);
		expect(codeTexts).toContain("markdown");
	});

	it("renders option description markdown as HTML", () => {
		render(
			<PermissionDialog
				request={mdAskRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
				onAnswer={vi.fn()}
			/>,
		);
		const dialog = screen.getByTestId("permission-dialog");
		expect(dialog.querySelector("strong")).toBeTruthy();
		expect(dialog.querySelector("strong")?.textContent).toBe("remark-gfm");
	});

	it("renders resolved question markdown as HTML", async () => {
		render(
			<PermissionDialog
				request={mdAskRequest}
				status="allowed"
				resolvedAnswers={{
					"Use `react-markdown` for rendering?": "Yes",
				}}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		const resolved = screen.getByTestId("permission-resolved");
		const button = resolved.querySelector("button");
		expect(button).toBeTruthy();
		await userEvent.click(button as HTMLElement);
		const codes = resolved.querySelectorAll("code");
		const codeTexts = Array.from(codes).map((c) => c.textContent);
		expect(codeTexts).toContain("react-markdown");
	});

	it("renders resolved answer as plain text (no markdown)", () => {
		const request = {
			...mdAskRequest,
			input: {
				questions: [
					{
						question: "Pick one",
						header: "Choice",
						options: [{ label: "A", description: "Option A" }],
						multiSelect: false,
					},
				],
			},
		};
		setPermissionPresentation(request);
		render(
			<PermissionDialog
				request={request}
				status="allowed"
				resolvedAnswers={{
					"Pick one": "Selected `option-A` with **bold**",
				}}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		const resolved = screen.getByTestId("permission-resolved");
		// 回答はユーザーの選択内容なのでプレーンテキスト表示（Markdown要素は生成しない）
		expect(resolved.querySelector("code")).toBeNull();
		expect(resolved.querySelector("strong")).toBeNull();
		expect(resolved.textContent).toContain("Selected `option-A` with **bold**");
	});
});
