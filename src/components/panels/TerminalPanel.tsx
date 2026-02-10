import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
	forwardRef,
	useCallback,
	useEffect,
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
import { quotePathForShell, quotePathsForShell } from "@/lib/quotePathForShell";
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
	const [isDragOver, setIsDragOver] = useState(false);

	useImperativeHandle(
		ref,
		() => ({
			writeToTerminal,
		}),
		[writeToTerminal],
	);

	// Tauri OS drag & drop (Finder等からのドラッグ)
	useEffect(() => {
		const unlisten = getCurrentWebview().onDragDropEvent((event) => {
			const container = containerRef.current;
			if (!container) return;

			const payload = event.payload;

			if (payload.type === "enter" || payload.type === "over") {
				const rect = container.getBoundingClientRect();
				const { x, y } = payload.position;
				const isInside =
					x >= rect.left &&
					x <= rect.right &&
					y >= rect.top &&
					y <= rect.bottom;
				setIsDragOver(isInside);
			} else if (payload.type === "drop") {
				const rect = container.getBoundingClientRect();
				const { x, y } = payload.position;
				const isInside =
					x >= rect.left &&
					x <= rect.right &&
					y >= rect.top &&
					y <= rect.bottom;
				if (isInside && payload.paths.length > 0) {
					writeToTerminal(quotePathsForShell(payload.paths));
				}
				setIsDragOver(false);
			} else if (payload.type === "leave") {
				setIsDragOver(false);
			}
		});

		return () => {
			unlisten.then((fn) => fn());
		};
	}, [writeToTerminal]);

	// アプリ内 HTML5 drag & drop (FileTreeからのドラッグ)
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
