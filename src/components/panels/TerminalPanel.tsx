import { forwardRef, useCallback, useImperativeHandle, useRef } from "react";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useTerminal } from "@/hooks/useTerminal";
import type { Theme } from "@/types/settings";
import "@xterm/xterm/css/xterm.css";

export interface TerminalPanelHandle {
	writeToTerminal: (data: string) => void;
}

export interface TerminalPanelProps {
	cwd?: string | null;
	theme?: Theme;
}

export const TerminalPanel = forwardRef<
	TerminalPanelHandle,
	TerminalPanelProps
>(function TerminalPanel({ cwd, theme }, ref) {
	const containerRef = useRef<HTMLDivElement>(null);
	const { terminalRef, writeToTerminal } = useTerminal(
		containerRef,
		cwd,
		theme,
	);

	useImperativeHandle(
		ref,
		() => ({
			writeToTerminal,
		}),
		[writeToTerminal],
	);

	const handleCopy = useCallback(async () => {
		const selection = terminalRef.current?.getSelection();
		if (selection) {
			await navigator.clipboard.writeText(selection);
		}
	}, [terminalRef]);

	const handlePaste = useCallback(async () => {
		const text = await navigator.clipboard.readText();
		terminalRef.current?.paste(text);
	}, [terminalRef]);

	const handleSelectAll = useCallback(() => {
		terminalRef.current?.selectAll();
	}, [terminalRef]);

	const handleClear = useCallback(() => {
		terminalRef.current?.clear();
	}, [terminalRef]);

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<div ref={containerRef} className="h-full w-full p-2 bg-terminal-bg" />
			</ContextMenuTrigger>
			<ContextMenuContent className="w-56">
				<ContextMenuItem onClick={handleCopy}>コピー</ContextMenuItem>
				<ContextMenuItem onClick={handlePaste}>貼り付け</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onClick={handleSelectAll}>全選択</ContextMenuItem>
				<ContextMenuItem onClick={handleClear}>クリア</ContextMenuItem>
			</ContextMenuContent>
		</ContextMenu>
	);
});
