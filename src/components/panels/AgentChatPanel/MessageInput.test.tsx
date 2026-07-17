import { invoke } from "@tauri-apps/api/core";
import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MessageInput, type MessageInputHandle } from "./MessageInput";
import { findMentionTrigger, findSkillTrigger } from "./popupInputUtils";

const mockInvoke = vi.mocked(invoke);

const defaultProps = {
	onSend: vi.fn().mockResolvedValue(true),
	onInterrupt: vi.fn(),
	isStreaming: false,
	mode: "edit" as const,
	onModeChange: vi.fn(),
	planMode: false,
	onPlanModeChange: vi.fn(),
	models: [{ value: "claude-opus-4-8" }],
	currentModelId: "claude-opus-4-8",
	onModelChange: vi.fn(),
	backends: [],
	currentBackendId: null,
	onBackendChange: vi.fn(),
	backendDisabled: true,
};

describe("MessageInput", () => {
	it("renders textarea and send button", () => {
		render(<MessageInput {...defaultProps} />);
		expect(screen.getByTestId("message-input")).toBeDefined();
		expect(screen.getByPlaceholderText("Send a message...")).toBeDefined();
		expect(screen.getByLabelText("Send message")).toBeDefined();
	});

	it("renders plan mode toggle", () => {
		render(<MessageInput {...defaultProps} />);
		expect(screen.getByTestId("plan-mode-toggle")).toBeDefined();
	});

	it("calls onPlanModeChange with the inverted plan mode when clicked", () => {
		const onPlanModeChange = vi.fn();
		render(
			<MessageInput
				{...defaultProps}
				planMode={true}
				onPlanModeChange={onPlanModeChange}
			/>,
		);

		fireEvent.click(screen.getByTestId("plan-mode-toggle"));

		expect(onPlanModeChange).toHaveBeenCalledWith(false);
	});

	it("uses unique plan mode ids and toggles only the matching instance from its label", async () => {
		const user = userEvent.setup();
		const onFirstPlanModeChange = vi.fn();
		const onSecondPlanModeChange = vi.fn();
		render(
			<>
				<MessageInput
					{...defaultProps}
					onPlanModeChange={onFirstPlanModeChange}
				/>
				<MessageInput
					{...defaultProps}
					onPlanModeChange={onSecondPlanModeChange}
				/>
			</>,
		);

		const labels = screen.getAllByText("Plan");
		const toggles = screen.getAllByTestId("plan-mode-toggle");
		expect(labels[0].getAttribute("for")).toBe(toggles[0].id);
		expect(labels[1].getAttribute("for")).toBe(toggles[1].id);
		expect(toggles[0].id).not.toBe(toggles[1].id);

		await user.click(labels[1]);

		expect(onFirstPlanModeChange).not.toHaveBeenCalled();
		expect(onSecondPlanModeChange).toHaveBeenCalledWith(true);
	});

	it("disables send button when input is empty", () => {
		render(<MessageInput {...defaultProps} />);
		const button = screen.getByLabelText("Send message");
		expect(button.hasAttribute("disabled")).toBe(true);
	});

	it("enables send button when input has content", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		const button = screen.getByLabelText("Send message");
		expect(button.hasAttribute("disabled")).toBe(false);
	});

	it("calls onSend when send button is clicked", () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(onSend).toHaveBeenCalledWith("Hello", undefined, undefined);
	});

	it("clears input only after sending succeeds", async () => {
		let resolveSend: ((value: boolean) => void) | undefined;
		const onSend = vi.fn(
			() =>
				new Promise<boolean>((resolve) => {
					resolveSend = resolve;
				}),
		);
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(textarea.value).toBe("Hello");

		await act(async () => resolveSend?.(true));
		await waitFor(() => expect(textarea.value).toBe(""));
	});

	it("preserves input when sending fails or rejects", async () => {
		const sendError = new Error("send failed");
		const consoleError = vi
			.spyOn(console, "error")
			.mockImplementation(() => {});
		const onSend = vi
			.fn()
			.mockResolvedValueOnce(false)
			.mockRejectedValueOnce(sendError);
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "Keep this" } });

		fireEvent.click(screen.getByLabelText("Send message"));
		await waitFor(() => expect(onSend).toHaveBeenCalledTimes(1));
		expect(textarea.value).toBe("Keep this");

		fireEvent.click(screen.getByLabelText("Send message"));
		await waitFor(() => expect(onSend).toHaveBeenCalledTimes(2));
		expect(textarea.value).toBe("Keep this");
		await waitFor(() =>
			expect(consoleError).toHaveBeenCalledWith(
				"Message send failed:",
				sendError,
			),
		);
		consoleError.mockRestore();
	});

	it("serializes submissions and preserves edited input and attachments added in flight", async () => {
		let resolveSend: ((value: boolean) => void) | undefined;
		const onSend = vi.fn(
			() =>
				new Promise<boolean>((resolve) => {
					resolveSend = resolve;
				}),
		);
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} onSend={onSend} ref={ref} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		const firstImage = { data: "Zmlyc3Q=", mediaType: "image/png" };
		const followUpImage = { data: "c2Vjb25k", mediaType: "image/png" };
		act(() => ref.current?.addImageAttachments([firstImage]));
		fireEvent.change(textarea, { target: { value: "First" } });

		fireEvent.click(screen.getByLabelText("Send message"));
		fireEvent.change(textarea, { target: { value: "FirstFollow-up" } });
		act(() => ref.current?.addImageAttachments([followUpImage]));
		fireEvent.click(screen.getByLabelText("Send message"));

		expect(onSend).toHaveBeenCalledTimes(1);
		expect(onSend).toHaveBeenCalledWith("First", [firstImage], undefined);

		await act(async () => resolveSend?.(true));
		await waitFor(() => expect(textarea.value).toBe("FirstFollow-up"));
		expect(screen.getAllByTestId("image-preview-item")).toHaveLength(1);
		expect(screen.getByAltText("Attached").getAttribute("src")).toBe(
			"data:image/png;base64,c2Vjb25k",
		);
	});

	it.each([
		{
			draftChanges: ["google"],
			expectedDraft: "google",
			caseName: "prefix replacement",
		},
		{
			draftChanges: ["cargo"],
			expectedDraft: "cargo",
			caseName: "suffix replacement",
		},
		{
			draftChanges: ["google", "go"],
			expectedDraft: "go",
			caseName: "replacement edited back to the submitted value",
		},
	])(
		"preserves a $caseName while sending",
		async ({ draftChanges, expectedDraft }) => {
			let resolveSend: ((value: boolean) => void) | undefined;
			const onSend = vi.fn(
				() =>
					new Promise<boolean>((resolve) => {
						resolveSend = resolve;
					}),
			);
			render(<MessageInput {...defaultProps} onSend={onSend} />);
			const textarea = screen.getByPlaceholderText(
				"Send a message...",
			) as HTMLTextAreaElement;
			fireEvent.change(textarea, { target: { value: "go" } });

			fireEvent.click(screen.getByLabelText("Send message"));
			for (const draft of draftChanges) {
				fireEvent.change(textarea, { target: { value: draft } });
			}

			await act(async () => resolveSend?.(true));
			await waitFor(() => expect(textarea.value).toBe(expectedDraft));
		},
	);

	it("sends on Cmd/Ctrl+Enter", () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		expect(onSend).toHaveBeenCalledWith("Hello", undefined, undefined);
	});

	it("does not send on Enter alone (Enter is reserved for newline)", () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.keyDown(textarea, { key: "Enter" });
		expect(onSend).not.toHaveBeenCalled();
	});

	it("does not send on Shift+Enter", () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });
		expect(onSend).not.toHaveBeenCalled();
	});

	it("shows interrupt button when streaming", () => {
		render(<MessageInput {...defaultProps} isStreaming={true} />);
		expect(screen.getByLabelText("Interrupt agent")).toBeDefined();
	});

	it("calls onInterrupt when interrupt button is clicked", () => {
		const onInterrupt = vi.fn();
		render(
			<MessageInput
				{...defaultProps}
				onInterrupt={onInterrupt}
				isStreaming={true}
			/>,
		);
		fireEvent.click(screen.getByLabelText("Interrupt agent"));
		expect(onInterrupt).toHaveBeenCalled();
	});

	it("keeps the interrupt button clickable while stopping", () => {
		const onInterrupt = vi.fn();
		render(
			<MessageInput
				{...defaultProps}
				onInterrupt={onInterrupt}
				isStreaming={true}
				isInterrupting={true}
			/>,
		);

		const button = screen.getByLabelText("Stopping agent");
		expect(button).not.toBeDisabled();
		fireEvent.click(button);
		expect(onInterrupt).toHaveBeenCalledOnce();
	});

	it("preserves draft text and attachments while stopping and re-interrupting", () => {
		const onInterrupt = vi.fn();
		const ref = createRef<MessageInputHandle>();
		const { rerender } = render(
			<MessageInput {...defaultProps} onInterrupt={onInterrupt} ref={ref} />,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "draft remains" } });
		act(() => {
			ref.current?.addImageAttachments([
				{ data: "aGVsbG8=", mediaType: "image/png" },
			]);
		});

		rerender(
			<MessageInput
				{...defaultProps}
				onInterrupt={onInterrupt}
				ref={ref}
				isStreaming={true}
				isInterrupting={true}
			/>,
		);
		fireEvent.click(screen.getByLabelText("Stopping agent"));

		expect(textarea.value).toBe("draft remains");
		expect(screen.getAllByTestId("image-preview-item")).toHaveLength(1);
		expect(onInterrupt).toHaveBeenCalledOnce();
	});

	it("interrupts with Ctrl+C while streaming and composer is empty", () => {
		const onInterrupt = vi.fn();
		render(
			<MessageInput
				{...defaultProps}
				onInterrupt={onInterrupt}
				isStreaming={true}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.keyDown(textarea, { key: "c", ctrlKey: true });
		expect(onInterrupt).toHaveBeenCalled();
	});

	it("does not interrupt with Ctrl+C when a queued follow-up is being edited", () => {
		const onInterrupt = vi.fn();
		render(
			<MessageInput
				{...defaultProps}
				onInterrupt={onInterrupt}
				isStreaming={true}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "queued follow-up" } });
		fireEvent.keyDown(textarea, { key: "c", ctrlKey: true });
		expect(onInterrupt).not.toHaveBeenCalled();
	});

	it("shows send button when streaming with text input", () => {
		render(<MessageInput {...defaultProps} isStreaming={true} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		expect(screen.getByText("Queue follow-up")).toBeInTheDocument();
		expect(screen.getByLabelText("Queue message")).toBeDefined();
		expect(screen.getByLabelText("Interrupt agent")).toBeDefined();
	});

	it("labels streaming Codex sends from backend capabilities", () => {
		render(
			<MessageInput
				{...defaultProps}
				currentBackendId="codex"
				isStreaming={true}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Add this constraint" } });
		expect(screen.getByText("Queue follow-up")).toBeInTheDocument();
		expect(screen.getByLabelText("Queue message")).toBeDefined();
		expect(screen.getByLabelText("Interrupt agent")).toBeDefined();
	});

	it("textarea is always enabled even during streaming", () => {
		render(<MessageInput {...defaultProps} isStreaming={true} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		expect(textarea.hasAttribute("disabled")).toBe(false);
	});

	it("calls onCycleMode on Shift+Tab", () => {
		const onCycleMode = vi.fn();
		render(<MessageInput {...defaultProps} onCycleMode={onCycleMode} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.keyDown(textarea, { key: "Tab", shiftKey: true });
		expect(onCycleMode).toHaveBeenCalled();
	});

	it("does not error when Shift+Tab pressed without onCycleMode", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		expect(() => {
			fireEvent.keyDown(textarea, { key: "Tab", shiftKey: true });
		}).not.toThrow();
	});

	it("accepts prompt suggestion with Tab when input is empty", () => {
		render(
			<MessageInput
				{...defaultProps}
				promptSuggestion="Continue with the next useful implementation step."
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Continue with the next useful implementation step.",
		) as HTMLTextAreaElement;

		fireEvent.keyDown(textarea, { key: "Tab" });

		expect(textarea.value).toBe(
			"Continue with the next useful implementation step.",
		);
	});

	it("does not send whitespace-only messages", () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "   " } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		expect(onSend).not.toHaveBeenCalled();
	});

	it("sends slash commands to the agent without native interception", async () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "/find agent bug" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		expect(onSend).toHaveBeenCalledWith(
			"/find agent bug",
			undefined,
			undefined,
		);
		await waitFor(() => expect(textarea.value).toBe(""));
	});

	it("sends unknown slash commands to the agent", () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/status" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		expect(onSend).toHaveBeenCalledWith("/status", undefined, undefined);
	});

	it("sends runtime slash commands to the SDK path without native handlers", async () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				runtimeSlashCommands={[{ name: "compact", description: "SDK compact" }]}
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "/compact native UX" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		expect(onSend).toHaveBeenCalledWith(
			"/compact native UX",
			undefined,
			undefined,
		);
		await waitFor(() => expect(textarea.value).toBe(""));
	});

	it("renders ModeSelector inside the input container", () => {
		render(<MessageInput {...defaultProps} />);
		expect(screen.getByTestId("mode-selector-trigger")).toBeDefined();
		expect(screen.getByTestId("mode-selector-trigger")).toHaveTextContent(
			"Edit",
		);
	});

	it("renders the same ModeSelector for the Codex backend (no backend-specific UI)", () => {
		render(
			<MessageInput
				{...defaultProps}
				currentBackendId="codex"
				backends={[
					{
						id: "claude",
						name: "Claude",
						available: true,
						availableModels: [],
					},
					{ id: "codex", name: "Codex", available: true, availableModels: [] },
				]}
			/>,
		);

		expect(screen.getByTestId("mode-selector-trigger")).toBeDefined();
	});

	it("renders ModelSelector inside the input container", () => {
		render(<MessageInput {...defaultProps} />);
		expect(screen.getByTestId("model-selector-trigger")).toBeDefined();
		expect(screen.getByTestId("model-selector-trigger")).toHaveTextContent(
			"claude-opus-4-8",
		);
	});

	it("renders ModelSelector with selected model value", () => {
		const models = [{ value: "claude-opus" }, { value: "claude-sonnet" }];
		render(
			<MessageInput
				{...defaultProps}
				models={models}
				currentModelId="claude-opus"
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toHaveTextContent(
			"claude-opus",
		);
	});

	it("renders enabled model selector when models are available and unlocked", () => {
		render(
			<MessageInput
				{...defaultProps}
				models={[
					{
						id: "claude:claude-opus-4-8",
						displayName: "Opus 4.8",
						backend: "claude",
						modelId: "claude-opus-4-8",
					},
				]}
				currentBackendId="claude"
				currentModelId="claude:claude-opus-4-8"
				canChangeBackend={true}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toHaveTextContent(
			"Opus 4.8",
		);
		expect(screen.getByTestId("model-selector-trigger")).toBeEnabled();
	});

	it("disables cross-backend model options after backend is locked", async () => {
		const user = userEvent.setup();
		render(
			<MessageInput
				{...defaultProps}
				models={[
					{
						id: "claude:claude-opus-4-8",
						displayName: "Opus 4.8",
						backend: "claude",
						modelId: "claude-opus-4-8",
					},
					{
						id: "codex:gpt-5.4",
						displayName: "GPT-5.4",
						backend: "codex",
						modelId: "gpt-5.4",
					},
				]}
				currentBackendId="claude"
				currentModelId="claude:claude-opus-4-8"
				canChangeBackend={false}
			/>,
		);
		await user.click(screen.getByTestId("model-selector-trigger"));
		expect(
			screen.getByText("GPT-5.4").closest("[role='menuitem']"),
		).toHaveAttribute("data-disabled");
	});
});

const slashCommands = [
	{ name: "plan-spec", description: "Create plan spec" },
	{ name: "plan-behavior", description: "Define behavior" },
	{ name: "review", description: "Code review", argumentHint: "<file>" },
	{ name: "commit", description: "Create a commit" },
];

describe("MessageInput slash command popup", () => {
	const renderWithRuntimeSlashCommands = () =>
		render(
			<MessageInput {...defaultProps} runtimeSlashCommands={slashCommands} />,
		);

	it("shows popup when typing /", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });
		expect(screen.getByTestId("slash-command-list")).toBeDefined();
		expect(screen.getAllByRole("option")).toHaveLength(4);
	});

	it("filters commands by prefix", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/pl" } });
		const options = screen.getAllByRole("option");
		expect(options).toHaveLength(2);
		expect(options[0].textContent).toContain("/plan-spec");
		expect(options[1].textContent).toContain("/plan-behavior");
	});

	it("dedupes runtime slash command names", () => {
		render(
			<MessageInput
				{...defaultProps}
				runtimeSlashCommands={[
					{ name: "compact", description: "Compact context" },
					{ name: "review", description: "SDK review" },
					{ name: "review", description: "Duplicate review" },
				]}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });
		const options = screen.getAllByRole("option");
		expect(options).toHaveLength(2);
		expect(options[0].textContent).toContain("/compact");
		expect(options[1].textContent).toContain("/review");
		expect(
			options.filter((option) => option.textContent?.includes("/review")),
		).toHaveLength(1);
	});

	it("does not show popup when value contains space", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/plan-spec " } });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("does not show popup when value does not start with /", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "hello" } });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("selects command on Enter", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "/" } });
		fireEvent.keyDown(textarea, { key: "Enter" });
		expect(textarea.value).toBe("/plan-spec ");
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("shows argument help after selecting a slash command with an argument hint", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "/review" } });
		fireEvent.keyDown(textarea, { key: "Enter" });

		const help = screen.getByTestId("slash-argument-help");
		expect(help.textContent).toContain("/review");
		expect(help.textContent).toContain("<file>");
		expect(help.textContent).toContain("Code review");

		fireEvent.change(textarea, { target: { value: "/review src/main.rs" } });
		expect(screen.queryByTestId("slash-argument-help")).toBeNull();
	});

	it("selects command on Tab", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "/" } });
		fireEvent.keyDown(textarea, { key: "Tab" });
		expect(textarea.value).toBe("/plan-spec ");
	});

	it("navigates with ArrowDown and ArrowUp", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });

		fireEvent.keyDown(textarea, { key: "ArrowDown" });
		const options = screen.getAllByRole("option");
		expect(options[1].dataset.selected).toBe("true");

		fireEvent.keyDown(textarea, { key: "ArrowUp" });
		expect(options[0].dataset.selected).toBe("true");
	});

	it("wraps around on ArrowDown at end", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });

		for (let i = 0; i < 4; i++) {
			fireEvent.keyDown(textarea, { key: "ArrowDown" });
		}
		const options = screen.getAllByRole("option");
		expect(options[0].dataset.selected).toBe("true");
	});

	it("wraps around on ArrowUp at start", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });

		fireEvent.keyDown(textarea, { key: "ArrowUp" });
		const options = screen.getAllByRole("option");
		expect(options[3].dataset.selected).toBe("true");
	});

	it("dismisses popup on Escape", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });
		expect(screen.getByTestId("slash-command-list")).toBeDefined();

		fireEvent.keyDown(textarea, { key: "Escape" });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("re-opens popup after dismiss when input changes", () => {
		renderWithRuntimeSlashCommands();
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });
		fireEvent.keyDown(textarea, { key: "Escape" });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();

		fireEvent.change(textarea, { target: { value: "/r" } });
		expect(screen.getByTestId("slash-command-list")).toBeDefined();
	});

	it("sends with Cmd+Enter even when popup is open", () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				runtimeSlashCommands={slashCommands}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/review" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		expect(onSend).toHaveBeenCalledWith("/review", undefined, undefined);
	});

	it("does not show popup without runtime slash commands", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});
});

const sampleAttachment = {
	data: "aGVsbG8=",
	mediaType: "image/png",
};

describe("MessageInput image attachments", () => {
	it("shows image preview when images are added via ref", () => {
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		expect(screen.getByTestId("image-preview-list")).toBeDefined();
		expect(screen.getAllByTestId("image-preview-item")).toHaveLength(1);
		const img = screen.getByAltText("Attached");
		expect(img.getAttribute("src")).toBe("data:image/png;base64,aGVsbG8=");
	});

	it("enables send button when only images are attached (no text)", () => {
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		const button = screen.getByLabelText("Send message");
		expect(button.hasAttribute("disabled")).toBe(false);
	});

	it("removes image when remove button is clicked", () => {
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		expect(screen.getAllByTestId("image-preview-item")).toHaveLength(1);
		fireEvent.click(screen.getByTestId("remove-image-button"));
		expect(screen.queryByTestId("image-preview-item")).toBeNull();
	});

	it("sends images with onSend when images are attached", () => {
		const onSend = vi.fn().mockResolvedValue(true);
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} onSend={onSend} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Check this" } });
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(onSend).toHaveBeenCalledWith(
			"Check this",
			[sampleAttachment],
			undefined,
		);
	});

	it("sends images only (no text) when text is empty", () => {
		const onSend = vi.fn().mockResolvedValue(true);
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} onSend={onSend} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(onSend).toHaveBeenCalledWith("", [sampleAttachment], undefined);
	});

	it("clears image preview after sending", async () => {
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		expect(screen.getByTestId("image-preview-list")).toBeDefined();
		fireEvent.click(screen.getByLabelText("Send message"));
		await waitFor(() =>
			expect(screen.queryByTestId("image-preview-list")).toBeNull(),
		);
	});

	it("preserves attached images when sending fails", async () => {
		const onSend = vi.fn().mockResolvedValue(false);
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} onSend={onSend} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		fireEvent.change(screen.getByPlaceholderText("Send a message..."), {
			target: { value: "Keep the image" },
		});

		fireEvent.click(screen.getByLabelText("Send message"));

		await waitFor(() => expect(onSend).toHaveBeenCalledTimes(1));
		expect(screen.getByTestId("image-preview-list")).toBeDefined();
		expect(onSend).toHaveBeenCalledWith(
			"Keep the image",
			[sampleAttachment],
			undefined,
		);
	});

	it("supports multiple image attachments", () => {
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([
				sampleAttachment,
				{ data: "aW1nMg==", mediaType: "image/jpeg" },
			]);
		});
		expect(screen.getAllByTestId("image-preview-item")).toHaveLength(2);
	});

	it("ignores non-image files on paste", async () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		const file = new File(["hello"], "readme.txt", { type: "text/plain" });
		const clipboardData = {
			items: [
				{
					type: "text/plain",
					getAsFile: () => file,
				},
			],
		};
		await act(async () => {
			fireEvent.paste(textarea, { clipboardData });
		});
		expect(screen.queryByTestId("image-preview-list")).toBeNull();
	});

	it("adds image from clipboard paste", async () => {
		vi.mocked(invoke).mockResolvedValueOnce(sampleAttachment);
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		const file = new File(["fake-png-data"], "screenshot.png", {
			type: "image/png",
		});
		const clipboardData = {
			items: [
				{
					type: "image/png",
					getAsFile: () => file,
				},
			],
		};
		await act(async () => {
			fireEvent.paste(textarea, { clipboardData });
		});
		await waitFor(() => {
			expect(invoke).toHaveBeenCalledWith("prepare_image_attachment", {
				data: expect.any(Array),
			});
		});
		expect(await screen.findByTestId("image-preview-list")).toBeDefined();
	});

	it("asks Rust whether text paste should collapse", async () => {
		const shortText = "short paste";
		mockInvoke.mockImplementation((command, args) => {
			if (command === "prepare_pasted_text_block") {
				expect(args).toEqual({ index: 1, content: shortText });
				return Promise.resolve(null);
			}
			return Promise.resolve([]);
		});
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		const clipboardData = {
			items: [],
			getData: (type: string) => (type === "text/plain" ? shortText : ""),
		};

		await act(async () => {
			fireEvent.paste(textarea, { clipboardData });
		});

		await waitFor(() => expect(textarea.value).toBe(shortText));
		expect(mockInvoke).toHaveBeenCalledWith("prepare_pasted_text_block", {
			index: 1,
			content: shortText,
		});
	});

	it("collapses long text paste and expands it through Rust on send", async () => {
		const longText = Array.from({ length: 24 }, (_, i) => `line ${i + 1}`).join(
			"\n",
		);
		const block = {
			id: 1,
			placeholder: "[Pasted text #1]",
			content: longText,
		};
		const onSend = vi.fn().mockResolvedValue(true);
		mockInvoke.mockImplementation((command, args) => {
			if (command === "prepare_pasted_text_block") {
				expect(args).toEqual({ index: 1, content: longText });
				return Promise.resolve(block);
			}
			if (command === "expand_pasted_text_blocks") {
				expect(args).toEqual({
					content: "[Pasted text #1]",
					blocks: [block],
				});
				return Promise.resolve("expanded prompt");
			}
			return Promise.resolve([]);
		});
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		const clipboardData = {
			items: [],
			getData: (type: string) => (type === "text/plain" ? longText : ""),
		};

		await act(async () => {
			fireEvent.paste(textarea, { clipboardData });
		});

		await waitFor(() => expect(textarea.value).toBe("[Pasted text #1]"));

		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		await waitFor(() =>
			expect(onSend).toHaveBeenCalledWith(
				"expanded prompt",
				undefined,
				undefined,
			),
		);
	});

	it("clears collapsed paste metadata after a successful send", async () => {
		const longText = Array.from({ length: 24 }, (_, i) => `line ${i + 1}`).join(
			"\n",
		);
		const block = {
			id: 1,
			placeholder: "[Pasted text #1]",
			content: longText,
		};
		const onSend = vi.fn().mockResolvedValue(true);
		const expand = vi.fn().mockResolvedValue("expanded prompt");
		mockInvoke.mockImplementation((command) => {
			if (command === "prepare_pasted_text_block") {
				return Promise.resolve(block);
			}
			if (command === "expand_pasted_text_blocks") {
				return expand();
			}
			return Promise.resolve([]);
		});
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		const clipboardData = {
			items: [],
			getData: (type: string) => (type === "text/plain" ? longText : ""),
		};

		await act(async () => {
			fireEvent.paste(textarea, { clipboardData });
		});
		await waitFor(() => expect(textarea.value).toBe(block.placeholder));
		fireEvent.click(screen.getByLabelText("Send message"));
		await waitFor(() => expect(textarea.value).toBe(""));

		fireEvent.change(textarea, { target: { value: "plain follow-up" } });
		fireEvent.click(screen.getByLabelText("Send message"));
		await waitFor(() => expect(onSend).toHaveBeenCalledTimes(2));

		expect(expand).toHaveBeenCalledTimes(1);
		expect(onSend).toHaveBeenLastCalledWith(
			"plain follow-up",
			undefined,
			undefined,
		);
	});

	it("preserves collapsed pasted text when sending fails", async () => {
		const longText = Array.from({ length: 24 }, (_, i) => `line ${i + 1}`).join(
			"\n",
		);
		const block = {
			id: 1,
			placeholder: "[Pasted text #1]",
			content: longText,
		};
		const onSend = vi.fn().mockResolvedValue(false);
		const expand = vi.fn().mockResolvedValue("expanded prompt");
		mockInvoke.mockImplementation((command) => {
			if (command === "prepare_pasted_text_block")
				return Promise.resolve(block);
			if (command === "expand_pasted_text_blocks") return expand();
			return Promise.resolve([]);
		});
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		const clipboardData = {
			items: [],
			getData: (type: string) => (type === "text/plain" ? longText : ""),
		};
		await act(async () => {
			fireEvent.paste(textarea, { clipboardData });
		});
		await waitFor(() => expect(textarea.value).toBe("[Pasted text #1]"));

		fireEvent.click(screen.getByLabelText("Send message"));
		await waitFor(() => expect(onSend).toHaveBeenCalledTimes(1));
		expect(textarea.value).toBe("[Pasted text #1]");

		fireEvent.click(screen.getByLabelText("Send message"));
		await waitFor(() => expect(onSend).toHaveBeenCalledTimes(2));
		expect(expand).toHaveBeenCalledTimes(2);
	});

	it("preserves collapsed paste metadata when the draft is edited in flight", async () => {
		const longText = Array.from({ length: 24 }, (_, i) => `line ${i + 1}`).join(
			"\n",
		);
		const block = {
			id: 1,
			placeholder: "[Pasted text #1]",
			content: longText,
		};
		let resolveFirstSend: ((value: boolean) => void) | undefined;
		const onSend = vi
			.fn()
			.mockImplementationOnce(
				() =>
					new Promise<boolean>((resolve) => {
						resolveFirstSend = resolve;
					}),
			)
			.mockResolvedValueOnce(true);
		const expand = vi.fn((content: string) =>
			Promise.resolve(
				content === block.placeholder
					? "expanded prompt"
					: "expanded prompt follow-up",
			),
		);
		mockInvoke.mockImplementation((command, args) => {
			if (command === "prepare_pasted_text_block") {
				return Promise.resolve(block);
			}
			if (command === "expand_pasted_text_blocks") {
				expect(args).toEqual({
					content: expect.any(String),
					blocks: [block],
				});
				return expand((args as { content: string }).content);
			}
			return Promise.resolve([]);
		});
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		const clipboardData = {
			items: [],
			getData: (type: string) => (type === "text/plain" ? longText : ""),
		};
		await act(async () => {
			fireEvent.paste(textarea, { clipboardData });
		});
		await waitFor(() => expect(textarea.value).toBe(block.placeholder));

		fireEvent.click(screen.getByLabelText("Send message"));
		await waitFor(() => expect(onSend).toHaveBeenCalledTimes(1));
		fireEvent.change(textarea, {
			target: { value: `${block.placeholder} follow-up` },
		});
		await act(async () => resolveFirstSend?.(true));
		await waitFor(() =>
			expect(textarea.value).toBe(`${block.placeholder} follow-up`),
		);

		fireEvent.click(screen.getByLabelText("Send message"));
		await waitFor(() => expect(onSend).toHaveBeenCalledTimes(2));
		expect(expand).toHaveBeenCalledTimes(2);
		expect(onSend).toHaveBeenLastCalledWith(
			"expanded prompt follow-up",
			undefined,
			undefined,
		);
	});
});

describe("findMentionTrigger", () => {
	it("returns null for empty text", () => {
		expect(findMentionTrigger("", 0)).toBeNull();
	});

	it("detects @ at start of text", () => {
		expect(findMentionTrigger("@src", 4)).toEqual({
			start: 0,
			query: "src",
		});
	});

	it("detects @ after whitespace", () => {
		expect(findMentionTrigger("hello @src", 10)).toEqual({
			start: 6,
			query: "src",
		});
	});

	it("returns null when @ is not preceded by whitespace", () => {
		expect(findMentionTrigger("hello@src", 9)).toBeNull();
	});

	it("returns null when query contains whitespace", () => {
		expect(findMentionTrigger("@ src", 5)).toBeNull();
	});

	it("allows whitespace inside quoted mention queries", () => {
		expect(findMentionTrigger('check @"docs/my file', 20)).toEqual({
			start: 6,
			query: "docs/my file",
		});
	});

	it("unescapes quoted mention query text", () => {
		expect(findMentionTrigger('@"docs/\\"guide', 14)).toEqual({
			start: 0,
			query: 'docs/"guide',
		});
	});

	it("preserves literal backslashes in quoted mention query text", () => {
		const value = '@"C:\\Users\\me\\file.ts';

		expect(findMentionTrigger(value, value.length)).toEqual({
			start: 0,
			query: "C:\\Users\\me\\file.ts",
		});
	});

	it("returns null after a quoted mention is closed", () => {
		expect(findMentionTrigger('check @"docs/my file" now', 22)).toBeNull();
	});

	it("returns empty query when cursor is right after @", () => {
		expect(findMentionTrigger("@", 1)).toEqual({ start: 0, query: "" });
	});

	it("handles cursor in middle of text", () => {
		expect(findMentionTrigger("hello @src/main.rs world", 18)).toEqual({
			start: 6,
			query: "src/main.rs",
		});
	});

	it("returns null when no @ in text", () => {
		expect(findMentionTrigger("hello world", 11)).toBeNull();
	});
});

describe("findSkillTrigger", () => {
	it("detects / at start of text", () => {
		expect(findSkillTrigger("/review", 7)).toEqual({
			start: 0,
			query: "review",
		});
	});

	it("returns null when / is not at the start of text", () => {
		expect(findSkillTrigger("use /review", 11)).toBeNull();
	});

	it("returns null when query contains whitespace", () => {
		expect(findSkillTrigger("/review extra", 13)).toBeNull();
	});

	it("returns empty query when cursor is right after /", () => {
		expect(findSkillTrigger("/", 1)).toEqual({ start: 0, query: "" });
	});
});

describe("MessageInput mention popup", () => {
	const mentionFiles = ["src/main.rs", "src/lib.rs", "src/app.tsx"];

	beforeEach(() => {
		vi.useFakeTimers();
		mockInvoke.mockImplementation((command) => {
			if (command === "list_mentionable_files") {
				return Promise.resolve(mentionFiles);
			}
			if (command === "sync_mentions_with_text") {
				return Promise.resolve(null);
			}
			return Promise.resolve([]);
		});
	});

	afterEach(() => {
		vi.useRealTimers();
		mockInvoke.mockReset();
	});

	it("shows mention popup when typing @ with worktreePath", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		expect(mockInvoke).toHaveBeenCalledWith("list_mentionable_files", {
			worktreePath: "/test/repo",
			query: "",
			backendId: undefined,
		});
		expect(screen.getByTestId("mention-file-list")).toBeDefined();
	});

	it("passes Codex backend id to unified mention popup search", async () => {
		mockInvoke.mockImplementation((command) => {
			if (command === "list_mentionable_files") {
				return Promise.resolve(["src/codex.rs"]);
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				currentBackendId="codex"
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@cod", selectionStart: 4 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));

		expect(mockInvoke).toHaveBeenCalledWith("list_mentionable_files", {
			worktreePath: "/test/repo",
			query: "cod",
			backendId: "codex",
		});
		expect(screen.getByText("src/codex.rs")).toBeDefined();
	});

	it("hides Codex mention popup when unified search fails", async () => {
		mockInvoke.mockImplementation((command) => {
			if (command === "list_mentionable_files") {
				return Promise.reject(new Error("codex unavailable"));
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				currentBackendId="codex"
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@fall", selectionStart: 5 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));

		expect(mockInvoke).toHaveBeenCalledWith("list_mentionable_files", {
			worktreePath: "/test/repo",
			query: "fall",
			backendId: "codex",
		});
		expect(screen.queryByTestId("mention-file-list")).toBeNull();
	});

	it("does not show mention popup without worktreePath", async () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		expect(screen.queryByTestId("mention-file-list")).toBeNull();
	});

	it("selects mention on Enter", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		expect(textarea.value).toBe("@src/main.rs ");
	});

	it("selects mention on Tab", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Tab" });
		expect(textarea.value).toBe("@src/main.rs ");
	});

	it("returns focus to textarea after mention selection", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		expect(document.activeElement).toBe(textarea);
	});

	it("dismisses mention popup on Escape", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		expect(screen.getByTestId("mention-file-list")).toBeDefined();
		fireEvent.keyDown(textarea, { key: "Escape" });
		expect(screen.queryByTestId("mention-file-list")).toBeNull();
		expect((textarea as HTMLTextAreaElement).value).toBe("@");
	});

	it("navigates mention list with ArrowDown and ArrowUp", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "ArrowDown" });
		const options = screen.getAllByRole("option");
		expect(options[1].dataset.selected).toBe("true");
		fireEvent.keyDown(textarea, { key: "ArrowUp" });
		expect(options[0].dataset.selected).toBe("true");
	});

	it("filters mentions by query", async () => {
		mockInvoke.mockResolvedValue(["src/main.rs"]);
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@main", selectionStart: 5 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		expect(mockInvoke).toHaveBeenCalledWith("list_mentionable_files", {
			worktreePath: "/test/repo",
			query: "main",
			backendId: undefined,
		});
	});

	it("closes mention popup when @ is deleted via backspace", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;

		// Type @ to open popup
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		expect(screen.getByTestId("mention-file-list")).toBeDefined();

		// Delete @ via backspace
		fireEvent.change(textarea, {
			target: { value: "", selectionStart: 0 },
		});
		expect(screen.queryByTestId("mention-file-list")).toBeNull();
	});

	it("does not show mention popup when no files match", async () => {
		mockInvoke.mockResolvedValue([]);
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@nonexistent", selectionStart: 12 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		expect(screen.queryByTestId("mention-file-list")).toBeNull();
	});

	it("replaces @query with selected file path when filter text exists", async () => {
		mockInvoke.mockResolvedValue(["src/main.rs"]);
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "check @main", selectionStart: 11 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		expect(textarea.value).toBe("check @src/main.rs ");
	});

	it("replaces @query in middle of text preserving surrounding text", async () => {
		mockInvoke.mockResolvedValue(["src/lib.rs"]);
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "check @lib please", selectionStart: 10 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		expect(textarea.value).toBe("check @src/lib.rs please");
	});

	it("quotes selected mention paths that contain spaces", async () => {
		mockInvoke.mockResolvedValue(["docs/my file.md"]);
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "check @my", selectionStart: 9 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		expect(textarea.value).toBe('check @"docs/my file.md" ');
	});

	it("debounces invoke calls", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "@m", selectionStart: 2 },
		});
		fireEvent.change(textarea, {
			target: { value: "@ma", selectionStart: 3 },
		});
		fireEvent.change(textarea, {
			target: { value: "@mai", selectionStart: 4 },
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"list_mentionable_files",
			expect.anything(),
		);
		await act(() => vi.advanceTimersByTimeAsync(150));
		expect(mockInvoke).toHaveBeenCalledWith("list_mentionable_files", {
			worktreePath: "/test/repo",
			query: "mai",
			backendId: undefined,
		});
	});

	it("sends mentions with onSend after selecting a mention", async () => {
		const onSend = vi.fn().mockResolvedValue(true);
		mockInvoke.mockImplementation((command, args) => {
			if (command === "list_mentionable_files") {
				return Promise.resolve(mentionFiles);
			}
			if (command === "sync_mentions_with_text") {
				expect(args).toEqual({
					text: "@src/main.rs",
					refs: [
						{
							filePath: "src/main.rs",
							startLine: undefined,
							endLine: undefined,
						},
					],
				});
				return Promise.resolve([
					{
						filePath: "src/main.rs",
						startLine: undefined,
						endLine: undefined,
					},
				]);
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		await act(async () => {
			await Promise.resolve();
		});
		expect(onSend).toHaveBeenCalledWith("@src/main.rs", undefined, [
			{ filePath: "src/main.rs", startLine: undefined, endLine: undefined },
		]);
	});

	it("clears mention metadata after a successful send", async () => {
		const onSend = vi.fn().mockResolvedValue(true);
		const syncedMention = {
			filePath: "src/main.rs",
			startLine: undefined,
			endLine: undefined,
		};
		const syncMentions = vi.fn().mockResolvedValue([syncedMention]);
		mockInvoke.mockImplementation((command, args) => {
			if (command === "list_mentionable_files") {
				return Promise.resolve(mentionFiles);
			}
			if (command === "sync_mentions_with_text") {
				return syncMentions(args);
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		fireEvent.click(screen.getByLabelText("Send message"));
		await act(async () => {
			await Promise.resolve();
			await Promise.resolve();
		});
		expect(textarea.value).toBe("");

		fireEvent.change(textarea, {
			target: { value: "plain follow-up", selectionStart: 15 },
		});
		fireEvent.click(screen.getByLabelText("Send message"));
		await act(async () => {
			await Promise.resolve();
			await Promise.resolve();
		});

		expect(syncMentions).toHaveBeenCalledTimes(1);
		expect(onSend).toHaveBeenCalledTimes(2);
		expect(onSend).toHaveBeenLastCalledWith(
			"plain follow-up",
			undefined,
			undefined,
		);
	});

	it("preserves input metadata and logs the stage when mention sync rejects", async () => {
		const longText = Array.from({ length: 24 }, (_, i) => `line ${i + 1}`).join(
			"\n",
		);
		const block = {
			id: 1,
			placeholder: "[Pasted text #1]",
			content: longText,
		};
		const syncedMention = {
			filePath: "src/main.rs",
			startLine: undefined,
			endLine: undefined,
		};
		const syncError = new Error("mention sync failed");
		const syncMentions = vi
			.fn()
			.mockRejectedValueOnce(syncError)
			.mockResolvedValueOnce([syncedMention]);
		const expand = vi.fn().mockResolvedValue("expanded prompt @src/main.rs");
		const onSend = vi.fn().mockResolvedValue(true);
		const consoleError = vi
			.spyOn(console, "error")
			.mockImplementation(() => {});
		const ref = createRef<MessageInputHandle>();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "prepare_pasted_text_block") {
				return Promise.resolve(block);
			}
			if (command === "expand_pasted_text_blocks") {
				return expand(args);
			}
			if (command === "list_mentionable_files") {
				return Promise.resolve(mentionFiles);
			}
			if (command === "sync_mentions_with_text") {
				return syncMentions(args);
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				ref={ref}
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		act(() => ref.current?.addImageAttachments([sampleAttachment]));
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		await act(() => vi.advanceTimersByTimeAsync(0));
		await act(async () => {
			fireEvent.paste(textarea, {
				clipboardData: {
					items: [],
					getData: (type: string) => (type === "text/plain" ? longText : ""),
				},
			});
		});

		await act(async () => {
			fireEvent.click(screen.getByLabelText("Send message"));
			await Promise.resolve();
			await Promise.resolve();
			await Promise.resolve();
		});

		expect(onSend).not.toHaveBeenCalled();
		expect(textarea.value).toBe(`@src/main.rs ${block.placeholder}`);
		expect(screen.getByTestId("image-preview-list")).toBeDefined();
		expect(screen.getByLabelText("Send message").hasAttribute("disabled")).toBe(
			false,
		);
		expect(expand).toHaveBeenCalledTimes(1);
		expect(syncMentions).toHaveBeenCalledWith({
			text: "expanded prompt @src/main.rs",
			refs: [syncedMention],
		});
		expect(consoleError).toHaveBeenCalledWith(
			"Message pre-send processing failed:",
			syncError,
		);
		expect(screen.queryByText("mention sync failed")).toBeNull();

		await act(async () => {
			fireEvent.click(screen.getByLabelText("Send message"));
			await Promise.resolve();
			await Promise.resolve();
			await Promise.resolve();
		});
		expect(expand).toHaveBeenCalledTimes(2);
		expect(syncMentions).toHaveBeenCalledTimes(2);
		expect(onSend).toHaveBeenCalledWith(
			"expanded prompt @src/main.rs",
			[sampleAttachment],
			[syncedMention],
		);
		consoleError.mockRestore();
	});

	it("preserves mention references when sending fails", async () => {
		const onSend = vi.fn().mockResolvedValue(false);
		const syncedMention = {
			filePath: "src/main.rs",
			startLine: undefined,
			endLine: undefined,
		};
		mockInvoke.mockImplementation((command) => {
			if (command === "list_mentionable_files") {
				return Promise.resolve(mentionFiles);
			}
			if (command === "sync_mentions_with_text") {
				return Promise.resolve([syncedMention]);
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });

		fireEvent.click(screen.getByLabelText("Send message"));
		await act(async () => Promise.resolve());
		expect(textarea.value).toBe("@src/main.rs ");
		expect(onSend).toHaveBeenCalledWith("@src/main.rs", undefined, [
			syncedMention,
		]);

		fireEvent.click(screen.getByLabelText("Send message"));
		await act(async () => Promise.resolve());
		expect(onSend).toHaveBeenCalledTimes(2);
		expect(onSend).toHaveBeenLastCalledWith("@src/main.rs", undefined, [
			syncedMention,
		]);
	});

	it("preserves mention metadata when the draft is edited in flight", async () => {
		let resolveFirstSend: ((value: boolean) => void) | undefined;
		const onSend = vi
			.fn()
			.mockImplementationOnce(
				() =>
					new Promise<boolean>((resolve) => {
						resolveFirstSend = resolve;
					}),
			)
			.mockResolvedValueOnce(true);
		const syncedMention = {
			filePath: "src/main.rs",
			startLine: undefined,
			endLine: undefined,
		};
		const syncMentions = vi.fn().mockResolvedValue([syncedMention]);
		mockInvoke.mockImplementation((command, args) => {
			if (command === "list_mentionable_files") {
				return Promise.resolve(mentionFiles);
			}
			if (command === "sync_mentions_with_text") {
				return syncMentions(args);
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });

		await act(async () => {
			fireEvent.click(screen.getByLabelText("Send message"));
			await Promise.resolve();
			await Promise.resolve();
		});
		expect(onSend).toHaveBeenCalledTimes(1);
		fireEvent.change(textarea, {
			target: {
				value: "@src/main.rs follow-up",
				selectionStart: 22,
			},
		});
		await act(async () => resolveFirstSend?.(true));
		expect(textarea.value).toBe("@src/main.rs follow-up");

		await act(async () => {
			fireEvent.click(screen.getByLabelText("Send message"));
			await Promise.resolve();
			await Promise.resolve();
		});
		expect(syncMentions).toHaveBeenCalledTimes(2);
		expect(syncMentions).toHaveBeenLastCalledWith({
			text: "@src/main.rs follow-up",
			refs: [syncedMention],
		});
		expect(onSend).toHaveBeenLastCalledWith(
			"@src/main.rs follow-up",
			undefined,
			[syncedMention],
		);
	});

	it("sends quoted mention paths with spaces as structured mentions", async () => {
		const onSend = vi.fn().mockResolvedValue(true);
		mockInvoke.mockImplementation((command, args) => {
			if (command === "list_mentionable_files") {
				return Promise.resolve(["docs/my file.md"]);
			}
			if (command === "sync_mentions_with_text") {
				expect(args).toEqual({
					text: '@"docs/my file.md"',
					refs: [
						{
							filePath: "docs/my file.md",
							startLine: undefined,
							endLine: undefined,
						},
					],
				});
				return Promise.resolve([
					{
						filePath: "docs/my file.md",
						startLine: undefined,
						endLine: undefined,
					},
				]);
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@my", selectionStart: 3 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		await act(async () => {
			await Promise.resolve();
		});
		expect(onSend).toHaveBeenCalledWith('@"docs/my file.md"', undefined, [
			{
				filePath: "docs/my file.md",
				startLine: undefined,
				endLine: undefined,
			},
		]);
	});

	it("excludes deleted mentions from onSend", async () => {
		const onSend = vi.fn().mockResolvedValue(true);
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		expect(textarea.value).toBe("@src/main.rs ");
		fireEvent.change(textarea, {
			target: { value: "hello world", selectionStart: 11 },
		});
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		await act(async () => {
			await Promise.resolve();
		});
		expect(onSend).toHaveBeenCalledWith("hello world", undefined, undefined);
	});

	it("extracts line number from @filePath:L50 in text", async () => {
		const onSend = vi.fn().mockResolvedValue(true);
		mockInvoke.mockImplementation((command, args) => {
			if (command === "list_mentionable_files") {
				return Promise.resolve(mentionFiles);
			}
			if (command === "sync_mentions_with_text") {
				expect(args).toEqual({
					text: "@src/main.rs:L50 check this",
					refs: [
						{
							filePath: "src/main.rs",
							startLine: undefined,
							endLine: undefined,
						},
					],
				});
				return Promise.resolve([
					{ filePath: "src/main.rs", startLine: 50, endLine: undefined },
				]);
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		fireEvent.change(textarea, {
			target: {
				value: "@src/main.rs:L50 check this",
				selectionStart: 27,
			},
		});
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		await act(async () => {
			await Promise.resolve();
		});
		expect(onSend).toHaveBeenCalledWith(
			"@src/main.rs:L50 check this",
			undefined,
			[{ filePath: "src/main.rs", startLine: 50, endLine: undefined }],
		);
	});

	it("extracts line range from @filePath:L10-L20 in text", async () => {
		const onSend = vi.fn().mockResolvedValue(true);
		mockInvoke.mockImplementation((command, args) => {
			if (command === "list_mentionable_files") {
				return Promise.resolve(mentionFiles);
			}
			if (command === "sync_mentions_with_text") {
				expect(args).toEqual({
					text: "@src/main.rs:L10-L20 review",
					refs: [
						{
							filePath: "src/main.rs",
							startLine: undefined,
							endLine: undefined,
						},
					],
				});
				return Promise.resolve([
					{ filePath: "src/main.rs", startLine: 10, endLine: 20 },
				]);
			}
			return Promise.resolve([]);
		});
		render(
			<MessageInput
				{...defaultProps}
				onSend={onSend}
				worktreePath="/test/repo"
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, {
			target: { value: "@", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });
		fireEvent.change(textarea, {
			target: {
				value: "@src/main.rs:L10-L20 review",
				selectionStart: 27,
			},
		});
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		await act(async () => {
			await Promise.resolve();
		});
		expect(onSend).toHaveBeenCalledWith(
			"@src/main.rs:L10-L20 review",
			undefined,
			[{ filePath: "src/main.rs", startLine: 10, endLine: 20 }],
		);
	});
});

describe("MessageInput skill popup", () => {
	const skills = [
		{
			name: "review",
			description: "Review code changes",
			scope: "project" as const,
		},
		{
			name: "docs",
			description: "Write docs",
			scope: "personal" as const,
		},
	];

	beforeEach(() => {
		vi.useFakeTimers();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "scan_agent_skills") {
				const query =
					typeof (args as { query?: unknown })?.query === "string"
						? String((args as { query?: unknown }).query).toLowerCase()
						: "";
				const limit =
					typeof (args as { limit?: unknown })?.limit === "number"
						? Number((args as { limit?: unknown }).limit)
						: skills.length;
				return Promise.resolve(
					skills
						.filter(
							(skill) =>
								query.length === 0 ||
								skill.name.toLowerCase().includes(query) ||
								skill.description.toLowerCase().includes(query),
						)
						.slice(0, limit),
				);
			}
			return Promise.resolve(null);
		});
	});

	afterEach(() => {
		vi.useRealTimers();
		mockInvoke.mockReset();
	});

	it("shows skills from Rust when typing / with worktreePath", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText("Send a message...");

		fireEvent.change(textarea, {
			target: { value: "/", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));

		expect(mockInvoke).toHaveBeenCalledWith("scan_agent_skills", {
			cwd: "/test/repo",
			backendId: undefined,
			query: "",
			limit: 20,
		});
		expect(screen.getByTestId("skill-list")).toBeDefined();
		expect(screen.getByText("/review")).toBeInTheDocument();
	});

	it("passes Codex backend id to unified skill catalog", async () => {
		render(
			<MessageInput
				{...defaultProps}
				worktreePath="/test/repo"
				currentBackendId="codex"
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");

		fireEvent.change(textarea, {
			target: { value: "/", selectionStart: 1 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));

		expect(mockInvoke).toHaveBeenCalledWith("scan_agent_skills", {
			cwd: "/test/repo",
			backendId: "codex",
			query: "",
			limit: 20,
		});
	});

	it("passes skill query to the Rust scanner", async () => {
		mockInvoke.mockResolvedValue([skills[1]]);
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText("Send a message...");

		fireEvent.change(textarea, {
			target: { value: "/doc", selectionStart: 4 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));

		expect(mockInvoke).toHaveBeenCalledWith("scan_agent_skills", {
			cwd: "/test/repo",
			backendId: undefined,
			query: "doc",
			limit: 20,
		});
		expect(screen.getByText("/docs")).toBeInTheDocument();
		expect(screen.queryByText("/review")).toBeNull();
	});

	it("inserts selected skill token on Enter", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;

		fireEvent.change(textarea, {
			target: { value: "/rev", selectionStart: 4 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Enter" });

		expect(textarea.value).toBe("/review ");
	});

	it("inserts selected skill token on Tab", async () => {
		render(<MessageInput {...defaultProps} worktreePath="/test/repo" />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;

		fireEvent.change(textarea, {
			target: { value: "/doc", selectionStart: 4 },
		});
		await act(() => vi.advanceTimersByTimeAsync(150));
		fireEvent.keyDown(textarea, { key: "Tab" });

		expect(textarea.value).toBe("/docs ");
	});
});
