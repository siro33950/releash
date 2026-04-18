import { invoke } from "@tauri-apps/api/core";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setSlashCommands } from "@/hooks/useSlashCommands";
import { MessageInput, type MessageInputHandle } from "./MessageInput";

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
		expect(invoke).toHaveBeenCalledWith("prepare_image_attachment", {
			data: expect.any(Array),
		});
		expect(screen.getByTestId("image-preview-list")).toBeDefined();
	});
});
