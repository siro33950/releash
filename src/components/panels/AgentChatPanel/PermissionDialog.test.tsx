import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { PermissionDialog } from "./PermissionDialog";

const baseRequest = {
	request_id: "req-001",
	tool_name: "Edit",
	input: { file_path: "/src/index.ts" },
	tool_use_id: "toolu_001",
};

describe("PermissionDialog", () => {
	it("displays tool name when no title or display_name", () => {
		render(
			<PermissionDialog
				request={baseRequest}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		expect(screen.getByText("Permission required: Edit")).toBeInTheDocument();
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
					request_id: "req-exitplan-001",
					tool_name: "ExitPlanMode",
					input: {
						plan: "# My Plan\n\n- Step 1\n- Step 2",
						planFilePath: "/tmp/plan.md",
					},
					tool_use_id: "toolu_exit_001",
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
					request_id: "req-exitplan-002",
					tool_name: "ExitPlanMode",
					input: {
						plan: "Some plan",
						planFilePath: "/tmp/secret/plan.md",
					},
					tool_use_id: "toolu_exit_002",
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
					request_id: "req-exitplan-003",
					tool_name: "ExitPlanMode",
					input: {
						plan: "Plan text",
						allowedPrompts: [
							{ tool: "Bash", prompt: "run tests" },
							{ tool: "Bash", prompt: "install dependencies" },
						],
					},
					tool_use_id: "toolu_exit_003",
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
					request_id: "req-exitplan-004",
					tool_name: "ExitPlanMode",
					input: { plan: "Plan only" },
					tool_use_id: "toolu_exit_004",
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
					request_id: "req-exitplan-005",
					tool_name: "ExitPlanMode",
					input: { plan: "Plan" },
					tool_use_id: "toolu_exit_005",
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
					request_id: "req-exitplan-006",
					tool_name: "ExitPlanMode",
					input: { plan: "Plan" },
					tool_use_id: "toolu_exit_006",
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
	request_id: "req-ask-001",
	tool_name: "AskUserQuestion",
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
	tool_use_id: "toolu_ask_001",
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
});

describe("AskUserQuestion — multiSelect", () => {
	const multiSelectRequest = {
		request_id: "req-ask-multi-001",
		tool_name: "AskUserQuestion",
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
		tool_use_id: "toolu_ask_multi_001",
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
		request_id: "req-ask-md-001",
		tool_name: "AskUserQuestion",
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
		tool_use_id: "toolu_ask_md_001",
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

	it("renders resolved answer markdown as HTML", async () => {
		render(
			<PermissionDialog
				request={{
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
				}}
				status="allowed"
				resolvedAnswers={{
					"Pick one": "Selected `option-A` with **bold**",
				}}
				onAllow={vi.fn()}
				onDeny={vi.fn()}
			/>,
		);
		const resolved = screen.getByTestId("permission-resolved");
		const button = resolved.querySelector("button");
		expect(button).toBeTruthy();
		await userEvent.click(button as HTMLElement);
		expect(resolved.querySelector("code")).toBeTruthy();
		expect(resolved.querySelector("code")?.textContent).toBe("option-A");
		expect(resolved.querySelector("strong")).toBeTruthy();
		expect(resolved.querySelector("strong")?.textContent).toBe("bold");
	});
});
