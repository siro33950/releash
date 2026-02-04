import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalPanel } from "./TerminalPanel";

const mockUseTerminal = vi.fn();

vi.mock("@/hooks/useTerminal", () => ({
	useTerminal: (...args: unknown[]) => mockUseTerminal(...args),
}));

vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

describe("TerminalPanel", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("コンテナdivが正しいclassNameで描画される", () => {
		const { container } = render(<TerminalPanel />);

		const terminalContainer = container.querySelector(".h-full.w-full");
		expect(terminalContainer).toBeInTheDocument();
	});

	it("useTerminal が containerRef とともに呼び出される", () => {
		render(<TerminalPanel />);

		expect(mockUseTerminal).toHaveBeenCalledWith(
			expect.objectContaining({ current: expect.any(HTMLDivElement) }),
		);
	});
});
