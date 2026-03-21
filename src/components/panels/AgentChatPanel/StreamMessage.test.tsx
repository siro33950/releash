import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { MessageRole } from "@/types/session";
import { StreamMessage } from "./StreamMessage";

const human: MessageRole = "human";
const agent: MessageRole = "agent";
const system: MessageRole = "system";

describe("StreamMessage", () => {
	it("renders human message", () => {
		render(
			<StreamMessage content="Hello agent" role={human} isStreaming={false} />,
		);
		const el = screen.getByTestId("stream-message-human");
		expect(el).toBeDefined();
		expect(el.textContent).toContain("Hello agent");
	});

	it("renders agent message", () => {
		render(
			<StreamMessage content="Hello human" role={agent} isStreaming={false} />,
		);
		const el = screen.getByTestId("stream-message-agent");
		expect(el).toBeDefined();
		expect(el.textContent).toContain("Hello human");
	});

	it("shows streaming cursor for agent when streaming", () => {
		const { container } = render(
			<StreamMessage content="Processing..." role={agent} isStreaming={true} />,
		);
		const cursor = container.querySelector(".animate-pulse");
		expect(cursor).not.toBeNull();
	});

	it("does not show streaming cursor when not streaming", () => {
		const { container } = render(
			<StreamMessage content="Done" role={agent} isStreaming={false} />,
		);
		const cursor = container.querySelector("span.animate-pulse");
		expect(cursor).toBeNull();
	});

	it("renders markdown in agent messages", () => {
		render(
			<StreamMessage
				content="**bold text**"
				role={agent}
				isStreaming={false}
			/>,
		);
		const el = screen.getByTestId("stream-message-agent");
		const bold = el.querySelector("strong");
		expect(bold).not.toBeNull();
		expect(bold?.textContent).toBe("bold text");
	});

	it("renders code blocks in agent messages", () => {
		render(
			<StreamMessage
				content={"```javascript\nconsole.log('hello');\n```"}
				role={agent}
				isStreaming={false}
			/>,
		);
		const el = screen.getByTestId("stream-message-agent");
		const code = el.querySelector("code");
		expect(code).not.toBeNull();
	});

	it("renders human messages as plain text (no markdown)", () => {
		render(
			<StreamMessage content="**not bold**" role={human} isStreaming={false} />,
		);
		const el = screen.getByTestId("stream-message-human");
		expect(el.querySelector("strong")).toBeNull();
		expect(el.textContent).toContain("**not bold**");
	});

	it("does not show separator for any messages", () => {
		const { container: c1 } = render(
			<StreamMessage content="Hello" role={human} isStreaming={false} />,
		);
		const { container: c2 } = render(
			<StreamMessage content="Hello" role={agent} isStreaming={false} />,
		);
		expect(c1.querySelector(".border-t")).toBeNull();
		expect(c2.querySelector(".border-t")).toBeNull();
	});

	it("renders system message with info style", () => {
		render(
			<StreamMessage
				content="Not logged in"
				role={system}
				isStreaming={false}
			/>,
		);
		const el = screen.getByTestId("stream-message-system");
		expect(el).toBeDefined();
		expect(el.textContent).toContain("Not logged in");
	});

	it("does not show role label for system messages", () => {
		render(
			<StreamMessage
				content="System notice"
				role={system}
				isStreaming={false}
			/>,
		);
		const el = screen.getByTestId("stream-message-system");
		expect(el.textContent).toBe("System notice");
	});
});
