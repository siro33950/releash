import { type ITheme, Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import "../styles/terminal.css";
import { type RefObject, useEffect, useRef } from "react";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

const terminalTheme: ITheme = {
	background: "#1a1a1a",
	foreground: "#e0e0e0",
	cursor: "#e0e0e0",
	cursorAccent: "#1a1a1a",
	black: "#1a1a1a",
	red: "#ff5f56",
	green: "#27c93f",
	yellow: "#ffbd2e",
	blue: "#2ea6ff",
	magenta: "#d75fff",
	cyan: "#5fd7ff",
	white: "#e0e0e0",
	brightBlack: "#7f7f7f",
	brightRed: "#ff6e67",
	brightGreen: "#5af78e",
	brightYellow: "#f9f1a5",
	brightBlue: "#57c7ff",
	brightMagenta: "#ff6ac1",
	brightCyan: "#9aedfe",
	brightWhite: "#ffffff",
};

const CHAR_WIDTH_RATIO = 0.6;
const MIN_FONT_SIZE = 8;
const DEFAULT_LINE_HEIGHT = 1.0;
const FONT_FAMILY = 'Menlo, Monaco, "Courier New", monospace';

interface UseRemoteTerminalOptions {
	containerRef: RefObject<HTMLDivElement | null>;
	ptyId: number;
	ptyCols: number;
	send: (msg: WsMessage) => void;
	subscribe: Subscribe;
	visible: boolean;
}

function calculateFontSize(containerWidth: number, cols: number): number {
	const idealSize = containerWidth / (cols * CHAR_WIDTH_RATIO);
	return Math.max(MIN_FONT_SIZE, Math.floor(idealSize));
}

function calculateRows(containerHeight: number, fontSize: number): number {
	const cellHeight = fontSize * (DEFAULT_LINE_HEIGHT + 0.2);
	return Math.max(1, Math.floor(containerHeight / cellHeight));
}

export function useRemoteTerminal({
	containerRef,
	ptyId,
	ptyCols,
	send,
	subscribe,
	visible,
}: UseRemoteTerminalOptions) {
	const terminalRef = useRef<Terminal | null>(null);
	const ptyColsRef = useRef(ptyCols);
	const initialSentRef = useRef(false);
	ptyColsRef.current = ptyCols;

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;

		const containerWidth = container.clientWidth;
		const containerHeight = container.clientHeight;
		const fontSize = calculateFontSize(containerWidth, ptyCols);
		const needsHorizontalScroll =
			fontSize <= MIN_FONT_SIZE &&
			containerWidth < ptyCols * MIN_FONT_SIZE * CHAR_WIDTH_RATIO;

		if (needsHorizontalScroll) {
			container.style.overflowX = "auto";
		} else {
			container.style.overflowX = "";
		}

		const rows = calculateRows(containerHeight, fontSize);

		const terminal = new Terminal({
			cols: ptyCols,
			rows,
			cursorBlink: true,
			disableStdin: true,
			fontFamily: FONT_FAMILY,
			fontSize,
			lineHeight: DEFAULT_LINE_HEIGHT,
			theme: terminalTheme,
			scrollback: 5000,
		});

		terminal.open(container);
		terminalRef.current = terminal;

		const handleResize = () => {
			const w = container.clientWidth;
			const h = container.clientHeight;
			const currentCols = ptyColsRef.current;
			const newFontSize = calculateFontSize(w, currentCols);
			const newRows = calculateRows(h, newFontSize);

			const needsScroll =
				newFontSize <= MIN_FONT_SIZE &&
				w < currentCols * MIN_FONT_SIZE * CHAR_WIDTH_RATIO;
			container.style.overflowX = needsScroll ? "auto" : "";

			terminal.options.fontSize = newFontSize;
			terminal.resize(currentCols, Math.max(1, newRows));
		};

		const resizeObserver = new ResizeObserver(handleResize);
		resizeObserver.observe(container);

		const unsubscribe = subscribe((msg: WsMessage) => {
			if (msg.type === "pty_output" && msg.payload.pty_id === ptyId) {
				terminal.write(msg.payload.data);
			} else if (msg.type === "pty_exit" && msg.payload.pty_id === ptyId) {
				terminal.write(
					`\r\n\x1b[90m[Process exited with code ${msg.payload.exit_code ?? "unknown"}]\x1b[0m\r\n`,
				);
			} else if (msg.type === "pty_resize" && msg.payload.pty_id === ptyId) {
				const newCols = msg.payload.cols;
				const newRows = msg.payload.rows;
				ptyColsRef.current = newCols;

				const w = container.clientWidth;
				const h = container.clientHeight;
				const newFontSize = calculateFontSize(w, newCols);
				const fitRows = calculateRows(h, newFontSize);

				const needsScroll =
					newFontSize <= MIN_FONT_SIZE &&
					w < newCols * MIN_FONT_SIZE * CHAR_WIDTH_RATIO;
				container.style.overflowX = needsScroll ? "auto" : "";

				terminal.options.fontSize = newFontSize;
				terminal.resize(newCols, Math.max(1, Math.min(newRows, fitRows)));
			}
		});

		requestAnimationFrame(() => {
			terminal.refresh(0, terminal.rows - 1);
			if (!initialSentRef.current) {
				initialSentRef.current = true;
				send({ type: "pty_input", payload: { pty_id: ptyId, data: "\r" } });
			}
		});

		return () => {
			unsubscribe();
			resizeObserver.disconnect();
			container.style.overflowX = "";
			terminal.dispose();
			terminalRef.current = null;
		};
	}, [containerRef, ptyId, ptyCols, send, subscribe]);

	useEffect(() => {
		const terminal = terminalRef.current;
		if (!visible || !terminal) return;

		requestAnimationFrame(() => {
			terminal.refresh(0, terminal.rows - 1);
		});
	}, [visible]);

	return { terminalRef };
}
