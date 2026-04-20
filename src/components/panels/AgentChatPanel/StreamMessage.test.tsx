import { invoke } from "@tauri-apps/api/core";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageRole } from "@/types/session";
import { StreamMessage } from "./StreamMessage";

const mockInvoke = vi.mocked(invoke);

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
		mockInvoke.mockReset();
		const displayFixtures: Record<
			string,
			Array<{ type: string; value: string }>
		> = {
			"Hello agent": [{ type: "text", value: "Hello agent" }],
			"**not bold**": [{ type: "text", value: "**not bold**" }],
			"Check @src/main.rs for details": [
				{ type: "text", value: "Check " },
				{ type: "mention", value: "@src/main.rs" },
				{ type: "text", value: " for details" },
			],
			"Compare @src/a.rs and @src/b.rs:L1-L5": [
				{ type: "text", value: "Compare " },
				{ type: "mention", value: "@src/a.rs" },
				{ type: "text", value: " and " },
				{ type: "mention", value: "@src/b.rs:L1-L5" },
			],
			"No mentions here": [{ type: "text", value: "No mentions here" }],
		};
		mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
			if (cmd === "parse_display_mentions") {
				const { content } = args as { content: string };
				return displayFixtures[content] ?? [{ type: "text", value: content }];
			}
			return undefined;
		});
	});

	it("renders human message", async () => {
		render(<StreamMessage content="Hello agent" role={human} />);
		const el = screen.getByTestId("stream-message-human");
		expect(el).toBeDefined();
		await waitFor(() => {
			expect(el.textContent).toContain("Hello agent");
		});
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

	it("renders human messages as plain text (no markdown)", async () => {
		render(<StreamMessage content="**not bold**" role={human} />);
		const el = screen.getByTestId("stream-message-human");
		await waitFor(() => {
			expect(el.querySelector("strong")).toBeNull();
			expect(el.textContent).toContain("**not bold**");
		});
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

	it("renders images in human message when image parts provided", () => {
		const images = [
			{ type: "image" as const, data: "aGVsbG8=", mediaType: "image/png" },
		];
		render(<StreamMessage content="Check this" role={human} images={images} />);
		const el = screen.getByTestId("stream-message-human");
		const imgs = el.querySelectorAll("img");
		expect(imgs.length).toBe(1);
		expect(imgs[0].getAttribute("src")).toBe("data:image/png;base64,aGVsbG8=");
		expect(el.textContent).toContain("Check this");
	});

	it("renders human message without images when no image parts", () => {
		render(<StreamMessage content="Hello" role={human} />);
		const el = screen.getByTestId("stream-message-human");
		const imgs = el.querySelectorAll("img");
		expect(imgs.length).toBe(0);
		expect(el.textContent).toContain("Hello");
	});

	it("renders multiple images in human message", () => {
		const images = [
			{ type: "image" as const, data: "aW1nMQ==", mediaType: "image/png" },
			{
				type: "image" as const,
				data: "aW1nMg==",
				mediaType: "image/jpeg",
			},
		];
		render(<StreamMessage content="Two images" role={human} images={images} />);
		const el = screen.getByTestId("stream-message-human");
		const imgs = el.querySelectorAll("img");
		expect(imgs.length).toBe(2);
	});

	it("renders image-only human message (no text)", () => {
		const images = [
			{ type: "image" as const, data: "aGVsbG8=", mediaType: "image/png" },
		];
		render(<StreamMessage content="" role={human} images={images} />);
		const el = screen.getByTestId("stream-message-human");
		const imgs = el.querySelectorAll("img");
		expect(imgs.length).toBe(1);
	});

	it("renders @mention as badge in human messages", async () => {
		render(
			<StreamMessage content="Check @src/main.rs for details" role={human} />,
		);
		const el = screen.getByTestId("stream-message-human");
		await waitFor(() => {
			const badge = el.querySelector(".font-mono");
			expect(badge).not.toBeNull();
			expect(badge?.textContent).toBe("@src/main.rs");
		});
	});

	it("renders multiple @mentions as badges", async () => {
		render(
			<StreamMessage
				content="Compare @src/a.rs and @src/b.rs:L1-L5"
				role={human}
			/>,
		);
		const el = screen.getByTestId("stream-message-human");
		await waitFor(() => {
			const badges = el.querySelectorAll(".font-mono");
			expect(badges.length).toBe(2);
			expect(badges[0].textContent).toBe("@src/a.rs");
			expect(badges[1].textContent).toBe("@src/b.rs:L1-L5");
		});
	});

	it("renders human message without mentions as plain text", async () => {
		render(<StreamMessage content="No mentions here" role={human} />);
		const el = screen.getByTestId("stream-message-human");
		await waitFor(() => {
			expect(el.querySelectorAll(".font-mono").length).toBe(0);
			expect(el.textContent).toContain("No mentions here");
		});
	});

	it("renders table inside ScrollArea", () => {
		const markdown =
			"| Col1 | Col2 | Col3 |\n|------|------|------|\n| A | B | C |";
		render(<StreamMessage content={markdown} role={agent} />);
		const el = screen.getByTestId("stream-message-agent");
		const scrollArea = el.querySelector('[data-slot="scroll-area"]');
		expect(scrollArea).not.toBeNull();
		expect(scrollArea?.querySelector("table")).not.toBeNull();
	});

	it("does not render ScrollArea when message has no table", () => {
		render(<StreamMessage content="Plain text without table" role={agent} />);
		const el = screen.getByTestId("stream-message-agent");
		const scrollArea = el.querySelector('[data-slot="scroll-area"]');
		expect(scrollArea).toBeNull();
	});
});
