import {
	forwardRef,
	useCallback,
	useImperativeHandle,
	useRef,
	useState,
} from "react";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useTerminal } from "@/hooks/useTerminal";
import { quotePathForShell } from "@/lib/quotePathForShell";
import type { Theme } from "@/types/settings";
import "@xterm/xterm/css/xterm.css";

export interface TerminalPanelHandle {
	writeToTerminal: (data: string) => void;
}

export interface TerminalPanelProps {
	cwd?: string | null;
	theme?: Theme;
	terminalStartupCommand?: string;
	sessionKey?: string;
	agentType?: string;
}

export const TerminalPanel = forwardRef<
	TerminalPanelHandle,
	TerminalPanelProps
>(function TerminalPanel(
	{ cwd, theme, terminalStartupCommand, sessionKey, agentType },
	ref,
) {
	const containerRef = useRef<HTMLDivElement>(null);
	const { terminalRef, writeToTerminal } = useTerminal(
		containerRef,
		cwd,
		theme,
		terminalStartupCommand,
		sessionKey,
		agentType,
	);
	const [isDragOver, setIsDragOver] = useState(false);

	useImperativeHandle(
		ref,
		() => ({
			writeToTerminal,
		}),
		[writeToTerminal],
	);

	// HTML5 drag & drop (アプリ内FileTreeからのドラッグ)
	// 外部ファイルドロップ(Finder等)はdragDropEnabled: falseではフルパス取得不可のため未対応
	const handleDragOver = useCallback((e: React.DragEvent) => {
		if (e.dataTransfer.types.includes("application/x-releash-file-path")) {
			e.preventDefault();
			e.dataTransfer.dropEffect = "copy";
			setIsDragOver(true);
		}
	}, []);

	const handleDragLeave = useCallback((e: React.DragEvent) => {
		if (!e.currentTarget.contains(e.relatedTarget as Node)) {
			setIsDragOver(false);
		}
	}, []);

	const handleDrop = useCallback(
		(e: React.DragEvent) => {
			e.preventDefault();
			setIsDragOver(false);
			const filePath = e.dataTransfer.getData(
				"application/x-releash-file-path",
			);
			if (filePath) {
				writeToTerminal(quotePathForShell(filePath));
			}
		},
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
				<div
					role="application"
					className="relative h-full w-full"
					onDragOver={handleDragOver}
					onDragLeave={handleDragLeave}
					onDrop={handleDrop}
				>
					<div
						ref={containerRef}
						className="h-full w-full p-2 bg-terminal-bg"
					/>
					{isDragOver && (
						<div className="absolute inset-0 flex items-center justify-center bg-primary/10 border-2 border-dashed border-primary rounded pointer-events-none">
							<span className="text-sm font-medium text-primary bg-background/80 px-3 py-1.5 rounded">
								ドロップしてパスを入力
							</span>
						</div>
					)}
				</div>
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
