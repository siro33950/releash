import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ChatMessage } from "@/types/session";
import { StreamMessage } from "./StreamMessage";

function makeMessage(role: "human" | "agent", content: string): ChatMessage {
	return {
		id: `msg-${role}-1`,
		role,
		content,
		timestamp: 1000,
	};
}

describe("StreamMessage", () => {
	it("renders human message with User label", () => {
		const msg = makeMessage("human", "Hello agent");
		render(<StreamMessage message={msg} isStreaming={false} />);
		const el = screen.getByTestId("stream-message-human");
		expect(el).toBeDefined();
		expect(el.textContent).toContain("User");
		expect(el.textContent).toContain("Hello agent");
	});

	it("renders agent message with Agent label", () => {
		const msg = makeMessage("agent", "Hello human");
		render(<StreamMessage message={msg} isStreaming={false} />);
		const el = screen.getByTestId("stream-message-agent");
		expect(el).toBeDefined();
		expect(el.textContent).toContain("Agent");
		expect(el.textContent).toContain("Hello human");
	});

	it("shows streaming cursor for agent when streaming", () => {
		const msg = makeMessage("agent", "Processing...");
		const { container } = render(
			<StreamMessage message={msg} isStreaming={true} />,
		);
		const cursor = container.querySelector(".animate-pulse");
		expect(cursor).not.toBeNull();
	});

	it("does not show streaming cursor when not streaming", () => {
		const msg = makeMessage("agent", "Done");
		const { container } = render(
			<StreamMessage message={msg} isStreaming={false} />,
		);
		const cursor = container.querySelector("span.animate-pulse");
		expect(cursor).toBeNull();
	});

	it("renders markdown in agent messages", () => {
		const msg = makeMessage("agent", "**bold text**");
		render(<StreamMessage message={msg} isStreaming={false} />);
		const el = screen.getByTestId("stream-message-agent");
		const bold = el.querySelector("strong");
		expect(bold).not.toBeNull();
		expect(bold?.textContent).toBe("bold text");
	});

	it("renders code blocks in agent messages", () => {
		const msg = makeMessage(
			"agent",
			"```javascript\nconsole.log('hello');\n```",
		);
		render(<StreamMessage message={msg} isStreaming={false} />);
		const el = screen.getByTestId("stream-message-agent");
		const code = el.querySelector("code");
		expect(code).not.toBeNull();
	});

	it("renders human messages as plain text (no markdown)", () => {
		const msg = makeMessage("human", "**not bold**");
		render(<StreamMessage message={msg} isStreaming={false} />);
		const el = screen.getByTestId("stream-message-human");
		expect(el.querySelector("strong")).toBeNull();
		expect(el.textContent).toContain("**not bold**");
	});

	it("shows separator for human messages", () => {
		const msg = makeMessage("human", "Hello");
		const { container } = render(
			<StreamMessage message={msg} isStreaming={false} />,
		);
		const separator = container.querySelector(".border-t");
		expect(separator).not.toBeNull();
	});

	it("does not show separator for agent messages", () => {
		const msg = makeMessage("agent", "Hello");
		const { container } = render(
			<StreamMessage message={msg} isStreaming={false} />,
		);
		const separator = container.querySelector(".border-t");
		expect(separator).toBeNull();
	});
});
