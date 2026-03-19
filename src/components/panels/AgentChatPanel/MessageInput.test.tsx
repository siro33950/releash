import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MessageInput } from "./MessageInput";

describe("MessageInput", () => {
	it("renders textarea and send button", () => {
		render(
			<MessageInput
				onSend={vi.fn()}
				onInterrupt={vi.fn()}
				disabled={false}
				isStreaming={false}
			/>,
		);
		expect(screen.getByTestId("message-input")).toBeDefined();
		expect(screen.getByPlaceholderText("Send a message...")).toBeDefined();
		expect(screen.getByLabelText("Send message")).toBeDefined();
	});

	it("disables send button when input is empty", () => {
		render(
			<MessageInput
				onSend={vi.fn()}
				onInterrupt={vi.fn()}
				disabled={false}
				isStreaming={false}
			/>,
		);
		const button = screen.getByLabelText("Send message");
		expect(button.hasAttribute("disabled")).toBe(true);
	});

	it("enables send button when input has content", () => {
		render(
			<MessageInput
				onSend={vi.fn()}
				onInterrupt={vi.fn()}
				disabled={false}
				isStreaming={false}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		const button = screen.getByLabelText("Send message");
		expect(button.hasAttribute("disabled")).toBe(false);
	});

	it("calls onSend when send button is clicked", () => {
		const onSend = vi.fn();
		render(
			<MessageInput
				onSend={onSend}
				onInterrupt={vi.fn()}
				disabled={false}
				isStreaming={false}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(onSend).toHaveBeenCalledWith("Hello");
	});

	it("clears input after sending", () => {
		render(
			<MessageInput
				onSend={vi.fn()}
				onInterrupt={vi.fn()}
				disabled={false}
				isStreaming={false}
			/>,
		);
		const textarea = screen.getByPlaceholderText(
			"Send a message...",
		) as HTMLTextAreaElement;
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.click(screen.getByLabelText("Send message"));
		expect(textarea.value).toBe("");
	});

	it("sends on Cmd/Ctrl+Enter", () => {
		const onSend = vi.fn();
		render(
			<MessageInput
				onSend={onSend}
				onInterrupt={vi.fn()}
				disabled={false}
				isStreaming={false}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		expect(onSend).toHaveBeenCalledWith("Hello");
	});

	it("does not send on Shift+Enter", () => {
		const onSend = vi.fn();
		render(
			<MessageInput
				onSend={onSend}
				onInterrupt={vi.fn()}
				disabled={false}
				isStreaming={false}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "Hello" } });
		fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });
		expect(onSend).not.toHaveBeenCalled();
	});

	it("shows interrupt button when streaming", () => {
		render(
			<MessageInput
				onSend={vi.fn()}
				onInterrupt={vi.fn()}
				disabled={false}
				isStreaming={true}
			/>,
		);
		expect(screen.getByLabelText("Interrupt agent")).toBeDefined();
	});

	it("calls onInterrupt when interrupt button is clicked", () => {
		const onInterrupt = vi.fn();
		render(
			<MessageInput
				onSend={vi.fn()}
				onInterrupt={onInterrupt}
				disabled={false}
				isStreaming={true}
			/>,
		);
		fireEvent.click(screen.getByLabelText("Interrupt agent"));
		expect(onInterrupt).toHaveBeenCalled();
	});

	it("disables textarea when disabled prop is true", () => {
		render(
			<MessageInput
				onSend={vi.fn()}
				onInterrupt={vi.fn()}
				disabled={true}
				isStreaming={false}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		expect(textarea.hasAttribute("disabled")).toBe(true);
	});

	it("does not send whitespace-only messages", () => {
		const onSend = vi.fn();
		render(
			<MessageInput
				onSend={onSend}
				onInterrupt={vi.fn()}
				disabled={false}
				isStreaming={false}
			/>,
		);
		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "   " } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
		expect(onSend).not.toHaveBeenCalled();
	});
});
