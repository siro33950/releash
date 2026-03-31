import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageRole } from "@/types/session";
import { StreamMessage } from "./StreamMessage";

const mockOpenUrl = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/plugin-opener", () => ({
	openUrl: (...args: unknown[]) => mockOpenUrl(...args),
}));

const human: MessageRole = "human";
const agent: MessageRole = "agent";
const system: MessageRole = "system";

describe("StreamMessage", () => {
	beforeEach(() => {
		mockOpenUrl.mockClear();
	});

	it("renders human message", () => {
		render(<StreamMessage content="Hello agent" role={human} />);
		const el = screen.getByTestId("stream-message-human");
		expect(el).toBeDefined();
		expect(el.textContent).toContain("Hello agent");
	});

	it("renders agent message", () => {
		render(<StreamMessage content="Hello human" role={agent} />);
		const el = screen.getByTestId("stream-message-agent");
		expect(el).toBeDefined();
		expect(el.textContent).toContain("Hello human");
	});

	it("renders markdown in agent messages", () => {
		render(<StreamMessage content="**bold text**" role={agent} />);
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
			/>,
		);
		const el = screen.getByTestId("stream-message-agent");
		const code = el.querySelector("code");
		expect(code).not.toBeNull();
	});

	it("renders human messages as plain text (no markdown)", () => {
		render(<StreamMessage content="**not bold**" role={human} />);
		const el = screen.getByTestId("stream-message-human");
		expect(el.querySelector("strong")).toBeNull();
		expect(el.textContent).toContain("**not bold**");
	});

	it("does not show separator for any messages", () => {
		const { container: c1 } = render(
			<StreamMessage content="Hello" role={human} />,
		);
		const { container: c2 } = render(
			<StreamMessage content="Hello" role={agent} />,
		);
		expect(c1.querySelector(".border-t")).toBeNull();
		expect(c2.querySelector(".border-t")).toBeNull();
	});

	it("renders system message with info style", () => {
		render(<StreamMessage content="Not logged in" role={system} />);
		const el = screen.getByTestId("stream-message-system");
		expect(el).toBeDefined();
		expect(el.textContent).toContain("Not logged in");
	});

	it("does not show role label for system messages", () => {
		render(<StreamMessage content="System notice" role={system} />);
		const el = screen.getByTestId("stream-message-system");
		expect(el.textContent).toBe("System notice");
	});

	it("renders link in agent message with custom anchor that calls openUrl on click", async () => {
		const user = userEvent.setup();
		render(
			<StreamMessage
				content="Visit [Example](https://example.com) for details"
				role={agent}
			/>,
		);
		const link = screen.getByRole("link", { name: "Example" });
		expect(link).toBeDefined();
		expect(link.getAttribute("href")).toBe("https://example.com");

		await user.click(link);
		expect(mockOpenUrl).toHaveBeenCalledWith("https://example.com");
	});

	it("prevents default navigation when clicking a link in agent message", () => {
		render(
			<StreamMessage
				content="Check [Docs](https://docs.example.com)"
				role={agent}
			/>,
		);
		const link = screen.getByRole("link", { name: "Docs" });

		const notPrevented = fireEvent.click(link);
		expect(notPrevented).toBe(false);
		expect(mockOpenUrl).toHaveBeenCalledWith("https://docs.example.com");
	});
});
