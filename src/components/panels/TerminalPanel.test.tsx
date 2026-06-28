import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { act, fireEvent, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalPanel } from "./TerminalPanel";

const mockWriteToTerminal = vi.fn();
const mockTerminalRef = { current: null };
const mockPtyIdRef: { current: number | null } = { current: 7 };
const mockRequestKill = vi.fn();
const mockUseTerminal = vi.fn().mockReturnValue({
	terminalRef: mockTerminalRef,
	ptyIdRef: mockPtyIdRef,
	writeToTerminal: mockWriteToTerminal,
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
		mockPtyIdRef.current = 7;
		mockInvoke.mockResolvedValue(undefined);
		mockListen.mockResolvedValue(vi.fn());
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
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
		);
	});

	it("onPtyReady を useTerminal に中継する", () => {
		const onPtyReady = vi.fn();
		render(<TerminalPanel onPtyReady={onPtyReady} />);

		expect(mockUseTerminal).toHaveBeenCalledWith(
			expect.objectContaining({ current: expect.any(HTMLDivElement) }),
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
			onPtyReady,
			undefined,
			undefined,
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

		expect(mockWriteToTerminal).not.toHaveBeenCalled();
		expect(mockInvoke).toHaveBeenCalledWith("write_paths_to_pty", {
			ptyId: 7,
			paths: ["/tmp/my file.txt"],
		});
	});

	it("PTY id 未確定時は drop path を書き込まない", () => {
		mockPtyIdRef.current = null;
		const { container } = render(<TerminalPanel />);
		const dropTarget = container.querySelector('[role="application"]');

		fireEvent.drop(dropTarget as Element, {
			dataTransfer: {
				getData: () => "/tmp/file.txt",
			},
		});

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_paths_to_pty",
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

		expect(mockInvoke).toHaveBeenCalledWith("write_paths_to_pty", {
			ptyId: 7,
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
			"write_paths_to_pty",
			expect.anything(),
		);
	});
});
