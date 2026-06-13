import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageRole } from "@/types/session";
import { StreamMessage } from "./StreamMessage";

const mockOpenUrl = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/plugin-opener", () => ({
	openUrl: (...args: unknown[]) => mockOpenUrl(...args),
}));

vi.mock("@tanstack/react-virtual", () => ({
	useVirtualizer: ({
		count,
		estimateSize,
	}: {
		count: number;
		estimateSize: (index: number) => number;
	}) => {
		const visibleCount = Math.min(count, 24);
		return {
			getVirtualItems: () =>
				Array.from({ length: visibleCount }, (_, i) => {
					const size = estimateSize(i);
					return {
						index: i,
						key: i,
						start: i * size,
						size,
						end: (i + 1) * size,
					};
				}),
			getTotalSize: () => {
				let total = 0;
				for (let i = 0; i < count; i++) {
					total += estimateSize(i);
				}
				return total;
			},
		};
	},
}));

const { markdownSpy } = vi.hoisted(() => ({ markdownSpy: vi.fn() }));
// Wrap react-markdown so we can count how often it is invoked for each render.
// The wrapper still delegates to the real implementation so unrelated markdown
// assertions in this file (bold, code, links, tables …) keep working.
vi.mock("react-markdown", async (importOriginal) => {
	const orig = await importOriginal<typeof import("react-markdown")>();
	const Wrapped = (
		props: Parameters<(typeof orig)["default"]>[0],
	): ReturnType<(typeof orig)["default"]> => {
		markdownSpy(props);
		return orig.default(props);
	};
	return { ...orig, default: Wrapped };
});

const human: MessageRole = "human";
const agent: MessageRole = "agent";
const system: MessageRole = "system";

describe("StreamMessage", () => {
	beforeEach(() => {
		mockOpenUrl.mockClear();
		markdownSpy.mockClear();
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

	it("renders agent messages as plain raw text in raw mode", () => {
		render(<StreamMessage content="**not bold**" role={agent} rawMode />);
		const el = screen.getByTestId("stream-message-agent");
		expect(screen.getByTestId("agent-raw-message")).toBeInTheDocument();
		expect(el.querySelector("strong")).toBeNull();
		expect(el.textContent).toContain("**not bold**");
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
			<StreamMessage
				content="Check @src/main.rs for details"
				role={human}
				mentions={[{ filePath: "src/main.rs" }]}
			/>,
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
				mentions={[
					{ filePath: "src/a.rs" },
					{ filePath: "src/b.rs", startLine: 1, endLine: 5 },
				]}
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

	it("renders Japanese filename @mention as badge in human messages", async () => {
		render(
			<StreamMessage
				content="確認してください @docs/Gitフロー.md の内容"
				role={human}
				mentions={[{ filePath: "docs/Gitフロー.md" }]}
			/>,
		);
		const el = screen.getByTestId("stream-message-human");
		await waitFor(() => {
			const badge = el.querySelector(".font-mono");
			expect(badge).not.toBeNull();
			expect(badge?.textContent).toBe("@docs/Gitフロー.md");
		});
	});

	it("renders quoted @mention with spaces as badge in human messages", async () => {
		render(
			<StreamMessage
				content={'Check @"docs/my file.md":L3-L7 for details'}
				role={human}
				mentions={[{ filePath: "docs/my file.md", startLine: 3, endLine: 7 }]}
			/>,
		);
		const el = screen.getByTestId("stream-message-human");
		await waitFor(() => {
			const badge = el.querySelector(".font-mono");
			expect(badge).not.toBeNull();
			expect(badge?.textContent).toBe('@"docs/my file.md":L3-L7');
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

	it("renders table inside horizontal overflow container", () => {
		const markdown =
			"| Col1 | Col2 | Col3 |\n|------|------|------|\n| A | B | C |";
		render(<StreamMessage content={markdown} role={agent} />);
		const el = screen.getByTestId("stream-message-agent");
		const table = el.querySelector("table");
		expect(table).not.toBeNull();
		const wrapper = table?.parentElement;
		expect(wrapper).not.toBeNull();
		expect(wrapper?.className).toContain("overflow-x-auto");
	});

	it("does not render overflow container when message has no table", () => {
		render(<StreamMessage content="Plain text without table" role={agent} />);
		const el = screen.getByTestId("stream-message-agent");
		expect(el.querySelector(".overflow-x-auto")).toBeNull();
	});

	it("collapses very large agent markdown without invoking markdown until expanded", async () => {
		const user = userEvent.setup();
		const largeMarkdown = `${"# Large\n\n"}${"body line\n".repeat(260)}**tail**`;

		render(<StreamMessage content={largeMarkdown} role={agent} />);

		expect(
			screen.getByTestId("large-agent-message-collapsed"),
		).toBeInTheDocument();
		expect(screen.getByText(/Large message collapsed/)).toBeInTheDocument();
		expect(markdownSpy).not.toHaveBeenCalled();

		await user.click(screen.getByText("Show full message"));

		expect(markdownSpy).toHaveBeenCalled();
		expect(screen.getByText("Large")).toBeInTheDocument();
		expect(screen.getByText("tail")).toBeInTheDocument();
	});

	it("uses line virtualization after expanding extremely large agent output", async () => {
		const user = userEvent.setup();
		const hugeOutput = Array.from(
			{ length: 900 },
			(_, i) => `line ${i + 1}`,
		).join("\n");

		render(<StreamMessage content={hugeOutput} role={agent} />);

		expect(
			screen.getByTestId("large-agent-message-collapsed"),
		).toBeInTheDocument();
		expect(markdownSpy).not.toHaveBeenCalled();

		await user.click(screen.getByText("Show full message"));

		expect(
			screen.getByTestId("large-agent-message-virtualized"),
		).toBeInTheDocument();
		expect(
			screen.getByText(/Virtualized full message: 900 lines/),
		).toBeInTheDocument();
		expect(screen.getByText("line 1")).toBeInTheDocument();
		expect(screen.queryByText("line 900")).not.toBeInTheDocument();
		expect(markdownSpy).not.toHaveBeenCalled();
	});

	it("does not re-invoke react-markdown when parent re-renders with identical content/role/images/mentions", () => {
		// React.memo on StreamMessage should short-circuit identical-prop
		// re-renders so the markdown pipeline does not run again. We count
		// how often the wrapped Markdown component is invoked: it must equal
		// the mount-time count even after the parent re-renders.
		let parentRenderCount = 0;
		function Probe({ tick }: { tick: number }) {
			parentRenderCount += 1;
			void tick;
			return <StreamMessage content="stable content" role={agent} />;
		}

		const { rerender } = render(<Probe tick={1} />);
		const mountInvocations = markdownSpy.mock.calls.length;
		expect(mountInvocations).toBeGreaterThan(0);

		rerender(<Probe tick={2} />);
		rerender(<Probe tick={3} />);

		// Parent re-rendered, but the memoized child did not — so the markdown
		// renderer was not invoked again.
		expect(parentRenderCount).toBeGreaterThan(1);
		expect(markdownSpy.mock.calls.length).toBe(mountInvocations);
	});

	it("re-renders when content changes (memo lets through real updates)", () => {
		const { rerender } = render(<StreamMessage content="first" role={agent} />);
		expect(screen.getByTestId("stream-message-agent").textContent).toContain(
			"first",
		);
		rerender(<StreamMessage content="second" role={agent} />);
		expect(screen.getByTestId("stream-message-agent").textContent).toContain(
			"second",
		);
	});

	it("memo skips re-render when images are value-equal but new references", () => {
		// shallowEqualImages must compare by value so a parent re-render that
		// creates a fresh array with the same image data does not bust the memo.
		const initial = [
			{ type: "image" as const, data: "aGVsbG8=", mediaType: "image/png" },
		];
		const equalButNewRef = [
			{ type: "image" as const, data: "aGVsbG8=", mediaType: "image/png" },
		];
		const { rerender } = render(
			<StreamMessage content="stable" role={agent} images={initial} />,
		);
		const mountInvocations = markdownSpy.mock.calls.length;
		expect(mountInvocations).toBeGreaterThan(0);

		rerender(
			<StreamMessage content="stable" role={agent} images={equalButNewRef} />,
		);
		expect(markdownSpy.mock.calls.length).toBe(mountInvocations);
	});

	it("memo skips re-render when mentions are value-equal but new references", () => {
		// shallowEqualMentions compares filePath / startLine / endLine — a fresh
		// array with the same fields must not trigger a re-parse.
		const initial = [{ filePath: "src/a.rs", startLine: 1, endLine: 5 }];
		const equalButNewRef = [{ filePath: "src/a.rs", startLine: 1, endLine: 5 }];
		const { rerender } = render(
			<StreamMessage content="stable" role={agent} mentions={initial} />,
		);
		const mountInvocations = markdownSpy.mock.calls.length;
		expect(mountInvocations).toBeGreaterThan(0);

		rerender(
			<StreamMessage content="stable" role={agent} mentions={equalButNewRef} />,
		);
		expect(markdownSpy.mock.calls.length).toBe(mountInvocations);
	});

	it("memo re-renders when images value changes", () => {
		const initial = [
			{ type: "image" as const, data: "aGVsbG8=", mediaType: "image/png" },
		];
		const changed = [
			{ type: "image" as const, data: "Z29vZA==", mediaType: "image/png" },
		];
		const { rerender } = render(
			<StreamMessage content="stable" role={human} images={initial} />,
		);
		const el = screen.getByTestId("stream-message-human");
		expect(el.querySelector("img")?.getAttribute("src")).toBe(
			"data:image/png;base64,aGVsbG8=",
		);

		rerender(<StreamMessage content="stable" role={human} images={changed} />);
		expect(
			screen
				.getByTestId("stream-message-human")
				.querySelector("img")
				?.getAttribute("src"),
		).toBe("data:image/png;base64,Z29vZA==");
	});

	it("memo re-renders when mentions value changes", () => {
		// content は固定したまま mentions のみ切り替え、shallowEqualMentions の
		// 差分判定が単独で再描画を引き起こすことを担保する。content も同時に
		// 変えてしまうと content 差分だけで再描画されてしまい、mentions 比較が
		// 壊れていても素通りしてしまうため。
		const stableContent = "Check @src/a.rs and @src/b.rs";
		const initial = [{ filePath: "src/a.rs" }];
		const changed = [{ filePath: "src/b.rs" }];
		const { rerender } = render(
			<StreamMessage content={stableContent} role={human} mentions={initial} />,
		);
		expect(
			screen.getByTestId("stream-message-human").querySelector(".font-mono")
				?.textContent,
		).toBe("@src/a.rs");

		rerender(
			<StreamMessage content={stableContent} role={human} mentions={changed} />,
		);
		expect(
			screen.getByTestId("stream-message-human").querySelector(".font-mono")
				?.textContent,
		).toBe("@src/b.rs");
	});
});
