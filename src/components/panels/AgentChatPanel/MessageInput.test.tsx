import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setSlashCommands } from "@/hooks/useSlashCommands";
import { MessageInput } from "./MessageInput";

const defaultProps = {
	onSend: vi.fn(),
	onInterrupt: vi.fn(),
	isStreaming: false,
	mode: "acceptEdits" as const,
	onModeChange: vi.fn(),
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
