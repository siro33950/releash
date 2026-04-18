import { invoke } from "@tauri-apps/api/core";
import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setSlashCommands } from "@/hooks/useSlashCommands";
import { MessageInput, type MessageInputHandle } from "./MessageInput";
import { findMentionTrigger } from "./popupInputUtils";

const mockInvoke = vi.mocked(invoke);

const defaultProps = {
	onSend: vi.fn(),
	onInterrupt: vi.fn(),
	isStreaming: false,
	mode: "acceptEdits" as const,
	onModeChange: vi.fn(),
	models: [],
	currentModelId: null,
	onModelChange: vi.fn(),
};

describe("MessageInput", () => {
	it("renders textarea and send button", () => {
		render(<MessageInput {...defaultProps} />);
		expect(screen.getByTestId("message-input")).toBeDefined();
		expect(screen.getByPlaceholderText("Send a message...")).toBeDefined();
		expect(screen.getByLabelText("Send message")).toBeDefined();
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
		const onSend = vi.fn();
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(onSend).toHaveBeenCalledWith("Hello");
	});

	it("clears input after sending", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(textarea.value).toBe("");
	});

	it("sends on Cmd/Ctrl+Enter", () => {
		const onSend = vi.fn();
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		expect(onSend).toHaveBeenCalledWith("Hello");
	});

	it("does not send on Shift+Enter", () => {
		const onSend = vi.fn();
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

	it("shows send button when streaming with text input", () => {
		render(<MessageInput {...defaultProps} isStreaming={true} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		expect(screen.getByLabelText("Send message")).toBeDefined();
		expect(screen.queryByLabelText("Interrupt agent")).toBeNull();
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

	it("does not send whitespace-only messages", () => {
		const onSend = vi.fn();
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "   " } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		expect(onSend).not.toHaveBeenCalled();
	});

	it("renders ModeSelector inside the input container", () => {
		render(<MessageInput {...defaultProps} />);
		expect(screen.getByTestId("mode-selector-trigger")).toBeDefined();
		expect(screen.getByTestId("mode-selector-trigger")).toHaveTextContent(
			"Code",
		);
	});

	it("renders ModelSelector inside the input container", () => {
		render(<MessageInput {...defaultProps} />);
		expect(screen.getByTestId("model-selector-trigger")).toBeDefined();
		expect(screen.getByTestId("model-selector-trigger")).toHaveTextContent(
			"Auto",
		);
	});

	it("renders ModelSelector with selected model name", () => {
		const models = [
			{ value: "claude-opus", displayName: "Claude Opus" },
			{ value: "claude-sonnet", displayName: "Claude Sonnet" },
		];
		render(
			<MessageInput
				{...defaultProps}
				models={models}
				currentModelId="claude-opus"
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toHaveTextContent(
			"Claude Opus",
		);
	});
});

const slashCommands = [
	{ name: "plan-spec", description: "Create plan spec" },
	{ name: "plan-behavior", description: "Define behavior" },
	{ name: "review", description: "Code review", argumentHint: "<file>" },
	{ name: "commit", description: "Create a commit" },
];

describe("MessageInput slash command popup", () => {
	beforeEach(() => {
		setSlashCommands(slashCommands);
	});

	it("shows popup when typing /", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });
		expect(screen.getByTestId("slash-command-list")).toBeDefined();
		expect(screen.getAllByRole("option")).toHaveLength(4);
	});

	it("filters commands by prefix", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/pl" } });
		const options = screen.getAllByRole("option");
		expect(options).toHaveLength(2);
		expect(options[0].textContent).toContain("/plan-spec");
		expect(options[1].textContent).toContain("/plan-behavior");
	});

	it("does not show popup when value contains space", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/plan-spec " } });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("does not show popup when value does not start with /", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "hello" } });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("selects command on Enter", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "/" } });
		fireEvent.keyDown(textarea, { key: "Enter" });
		expect(textarea.value).toBe("/plan-spec ");
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("selects command on Tab", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "/" } });
		fireEvent.keyDown(textarea, { key: "Tab" });
		expect(textarea.value).toBe("/plan-spec ");
	});

	it("navigates with ArrowDown and ArrowUp", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });

		fireEvent.keyDown(textarea, { key: "ArrowDown" });
		const options = screen.getAllByRole("option");
		expect(options[1].dataset.selected).toBe("true");

		fireEvent.keyDown(textarea, { key: "ArrowUp" });
		expect(options[0].dataset.selected).toBe("true");
	});

	it("wraps around on ArrowDown at end", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });

		for (let i = 0; i < 4; i++) {
			fireEvent.keyDown(textarea, { key: "ArrowDown" });
		}
		const options = screen.getAllByRole("option");
		expect(options[0].dataset.selected).toBe("true");
	});

	it("wraps around on ArrowUp at start", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });

		fireEvent.keyDown(textarea, { key: "ArrowUp" });
		const options = screen.getAllByRole("option");
		expect(options[3].dataset.selected).toBe("true");
	});

	it("dismisses popup on Escape", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });
		expect(screen.getByTestId("slash-command-list")).toBeDefined();

		fireEvent.keyDown(textarea, { key: "Escape" });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("re-opens popup after dismiss when input changes", () => {
		render(<MessageInput {...defaultProps} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/" } });
		fireEvent.keyDown(textarea, { key: "Escape" });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();

		fireEvent.change(textarea, { target: { value: "/r" } });
		expect(screen.getByTestId("slash-command-list")).toBeDefined();
	});

	it("sends with Cmd+Enter even when popup is open", () => {
		const onSend = vi.fn();
		render(<MessageInput {...defaultProps} onSend={onSend} />);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "/review" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		expect(onSend).toHaveBeenCalledWith("/review");
	});

	it("does not show popup when commands cache is empty", () => {
		setSlashCommands([]);
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
		const onSend = vi.fn();
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} onSend={onSend} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Check this" } });
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(onSend).toHaveBeenCalledWith("Check this", [sampleAttachment]);
	});

	it("sends images only (no text) when text is empty", () => {
		const onSend = vi.fn();
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} onSend={onSend} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(onSend).toHaveBeenCalledWith("", [sampleAttachment]);
	});

	it("clears image preview after sending", () => {
		const ref = createRef<MessageInputHandle>();
		render(<MessageInput {...defaultProps} ref={ref} />);
		act(() => {
			ref.current?.addImageAttachments([sampleAttachment]);
		});
		expect(screen.getByTestId("image-preview-list")).toBeDefined();
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(screen.queryByTestId("image-preview-list")).toBeNull();
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

describe("MessageInput mention popup", () => {
	const mentionFiles = ["src/main.rs", "src/lib.rs", "src/app.tsx"];

	beforeEach(() => {
		vi.useFakeTimers();
		mockInvoke.mockResolvedValue(mentionFiles);
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
		});
		expect(screen.getByTestId("mention-file-list")).toBeDefined();
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
		});
	});
	});
});
