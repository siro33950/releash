import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalPanel } from "./TerminalPanel";

const mockSendInput = vi.fn();
const mockTerminalRef = { current: null };
const mockIsRunningRef = { current: true };
const mockTerminalOwner = { kind: "workspace", workspacePath: "" } as const;
const mockRequestKill = vi.fn();
const mockUseTerminal = vi.fn().mockReturnValue({
	terminalRef: mockTerminalRef,
	terminalOwner: mockTerminalOwner,
	isRunningRef: mockIsRunningRef,
	sendInput: mockSendInput,
	requestKill: mockRequestKill,
});
const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);

vi.mock("@/hooks/useTerminal", () => ({
	useTerminal: (...args: unknown[]) => mockUseTerminal(...args),
}));

vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

describe("TerminalPanel", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockIsRunningRef.current = true;
		mockInvoke.mockResolvedValue(undefined);
		mockListen.mockResolvedValue(vi.fn());
	});

	it("余白をxtermの計測対象外へ置きhostの高さを実表示領域に一致させる", () => {
		render(<TerminalPanel />);

		const surface = screen.getByRole("application");
		const terminalHost = surface.firstElementChild;
		expect(surface).toHaveClass("h-full", "w-full", "p-2");
		expect(terminalHost).toHaveClass("h-full", "w-full");
		expect(terminalHost).not.toHaveClass("p-2");
	});

	it("useTerminal が containerRef とともに呼び出される", () => {
		render(<TerminalPanel />);

		expect(mockUseTerminal).toHaveBeenCalledWith(
			expect.objectContaining({ current: expect.any(HTMLDivElement) }),
			expect.any(Object),
		);
	});

	it("onTerminalReady を useTerminal に中継する", () => {
		const onTerminalReady = vi.fn();
		render(<TerminalPanel onTerminalReady={onTerminalReady} />);

		expect(mockUseTerminal).toHaveBeenCalledWith(
			expect.objectContaining({ current: expect.any(HTMLDivElement) }),
			expect.objectContaining({ onTerminalReady }),
		);
	});

	it("drop された path を escaping せず Rust command に渡す", () => {
		const { container } = render(<TerminalPanel />);
		const dropTarget = container.querySelector('[role="application"]');

		expect(dropTarget).toBeInTheDocument();
		fireEvent.drop(dropTarget as Element, {
			dataTransfer: {
				getData: (type: string) =>
					type === "application/x-releash-file-path" ? "/tmp/my file.txt" : "",
			},
		});

		expect(mockSendInput).not.toHaveBeenCalled();
		expect(mockInvoke).toHaveBeenCalledWith("write_paths_to_terminal_surface", {
			owner: mockTerminalOwner,
			paths: ["/tmp/my file.txt"],
		});
	});

	it("Terminal process が実行中でない場合は drop path を書き込まない", () => {
		mockIsRunningRef.current = false;
		const { container } = render(<TerminalPanel />);
		const dropTarget = container.querySelector('[role="application"]');

		fireEvent.drop(dropTarget as Element, {
			dataTransfer: {
				getData: () => "/tmp/file.txt",
			},
		});

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_paths_to_terminal_surface",
			expect.anything(),
		);
	});

	it("native-file-drop は dragOver 後に複数 raw paths を Rust command に渡す", async () => {
		type NativeFileDropCallback = (event: {
			payload: { paths: string[] };
		}) => void;
		let nativeFileDropCallback: NativeFileDropCallback | null = null;
		mockListen.mockImplementation((event, callback) => {
			if (event === "native-file-drop") {
				nativeFileDropCallback = callback as NativeFileDropCallback;
			}
			return Promise.resolve(vi.fn());
		});
		const paths = ["/tmp/a.txt", "/tmp/my file.txt", "/tmp/b.txt"];
		const { container } = render(<TerminalPanel />);
		const dropTarget = container.querySelector('[role="application"]');

		expect(dropTarget).toBeInTheDocument();
		fireEvent.dragOver(dropTarget as Element, {
			dataTransfer: { types: ["Files"], dropEffect: "" },
		});
		await act(async () => {
			nativeFileDropCallback?.({ payload: { paths } });
		});

		expect(mockInvoke).toHaveBeenCalledWith("write_paths_to_terminal_surface", {
			owner: mockTerminalOwner,
			paths,
		});
	});

	it("native-file-drop は dragOver 前なら書き込まない", async () => {
		type NativeFileDropCallback = (event: {
			payload: { paths: string[] };
		}) => void;
		let nativeFileDropCallback: NativeFileDropCallback | null = null;
		mockListen.mockImplementation((event, callback) => {
			if (event === "native-file-drop") {
				nativeFileDropCallback = callback as NativeFileDropCallback;
			}
			return Promise.resolve(vi.fn());
		});
		render(<TerminalPanel />);

		await act(async () => {
			nativeFileDropCallback?.({ payload: { paths: ["/tmp/a.txt"] } });
		});

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_paths_to_terminal_surface",
			expect.anything(),
		);
	});
});
