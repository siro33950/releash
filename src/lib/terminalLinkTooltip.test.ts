import { afterEach, describe, expect, it, vi } from "vitest";
import { createTerminalLinkTooltip } from "./terminalLinkTooltip";

function createTarget(): HTMLElement {
	const target = document.createElement("div");
	document.body.append(target);
	return target;
}

describe("createTerminalLinkTooltip", () => {
	afterEach(() => {
		vi.restoreAllMocks();
		document.body.replaceChildren();
	});

	it("hoverで生成先要素の子にURL全体を表示する", () => {
		const target = createTarget();
		const tooltip = createTerminalLinkTooltip(target);
		const url =
			"https://example.com/a/terminal-width-wrapped/path?query=value#fragment";

		tooltip.hover(
			new MouseEvent("mousemove", { clientX: 40, clientY: 50 }),
			url,
		);

		const element = target.querySelector('[role="tooltip"]');
		expect(element?.parentElement).toBe(target);
		expect(element).toHaveTextContent(url);
	});

	it("terminal右下端のhoverでもtooltip全体を生成先要素内に配置する", () => {
		const target = createTarget();
		target.style.overflow = "hidden";
		const targetBounds = {
			left: 100,
			top: 50,
			width: 300,
			height: 200,
		} as DOMRect;
		const tooltipBounds = { width: 180, height: 48 } as DOMRect;
		vi.spyOn(target, "getBoundingClientRect").mockReturnValue(targetBounds);
		vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
			function () {
				return this.classList.contains("xterm-hover")
					? tooltipBounds
					: ({} as DOMRect);
			},
		);
		const tooltip = createTerminalLinkTooltip(target);
		const url =
			"https://example.com/a/terminal-width-wrapped/path?query=value#fragment";

		tooltip.hover(
			new MouseEvent("mousemove", { clientX: 390, clientY: 240 }),
			url,
		);

		const element = target.querySelector<HTMLElement>('[role="tooltip"]');
		expect(element).toHaveTextContent(url);
		expect(element?.style.left).toBe("108px");
		expect(element?.style.top).toBe("130px");
	});

	it("tooltip要素にxterm-hover classを付ける", () => {
		const target = createTarget();
		const tooltip = createTerminalLinkTooltip(target);

		tooltip.hover(new MouseEvent("mousemove"), "https://example.com");

		expect(target.querySelector('[role="tooltip"]')).toHaveClass("xterm-hover");
	});

	it("leaveでtooltipを消す", () => {
		const target = createTarget();
		const tooltip = createTerminalLinkTooltip(target);
		tooltip.hover(new MouseEvent("mousemove"), "https://example.com");

		tooltip.leave();

		expect(target.querySelector('[role="tooltip"]')).not.toBeInTheDocument();
	});

	it("disposeでhover状態と生成先要素の参照を解放する", () => {
		const target = createTarget();
		const tooltip = createTerminalLinkTooltip(target);
		tooltip.hover(new MouseEvent("mousemove"), "https://example.com/first");

		tooltip.dispose();
		tooltip.hover(new MouseEvent("mousemove"), "https://example.com/second");

		expect(target.querySelector('[role="tooltip"]')).not.toBeInTheDocument();
	});
});
