import { useCallback, useRef } from "react";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useTerminal } from "@/hooks/useTerminal";
import "@xterm/xterm/css/xterm.css";

export interface TerminalPanelProps {
	cwd?: string | null;
}

export function TerminalPanel({ cwd }: TerminalPanelProps) {
	const containerRef = useRef<HTMLDivElement>(null);
	const { terminalRef } = useTerminal(containerRef, cwd);

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
				<div ref={containerRef} className="h-full w-full p-2 bg-[#1a1a1a]" />
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
}
