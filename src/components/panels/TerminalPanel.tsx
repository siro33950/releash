import { listen } from "@tauri-apps/api/event";
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
import type { NativeFileDropPayload } from "@/hooks/useNativeFileDrop";
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
	terminalStartupCommand?: string;
	sessionKey?: string;
	agentType?: string;
	label?: string;
}

export const TerminalPanel = forwardRef<
	TerminalPanelHandle,
	TerminalPanelProps
>(function TerminalPanel(
	{ cwd, theme, terminalStartupCommand, sessionKey, agentType, label },
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
		label,
	);
	const [isDragOver, setIsDragOver] = useState(false);
	const isDragOverRef = useRef(false);

	useImperativeHandle(
		ref,
		() => ({
			writeToTerminal,
		}),
		[writeToTerminal],
	);

	// ネイティブドロップ時はHTML5のdropイベントが発火しないため、
	// isDragOverRefでドラッグ状態を追跡し、自身のターミナルにパスを書き込む
	useEffect(() => {
		const unlisten = listen<NativeFileDropPayload>(
			"native-file-drop",
			(event) => {
				if (isDragOverRef.current) {
					const { paths } = event.payload;
					if (paths.length > 0) {
						writeToTerminal(quotePathsForShell(paths));
					}
				}
				isDragOverRef.current = false;
				setIsDragOver(false);
			},
		);
		return () => {
			unlisten.then((f) => f());
		};
	}, [writeToTerminal]);

	const handleDragOver = useCallback((e: React.DragEvent) => {
		if (
			e.dataTransfer.types.includes("application/x-releash-file-path") ||
			e.dataTransfer.types.includes("Files")
		) {
			e.preventDefault();
			e.dataTransfer.dropEffect = "copy";
			isDragOverRef.current = true;
			setIsDragOver(true);
		}
	}, []);

	const handleDragLeave = useCallback((e: React.DragEvent) => {
		if (!e.currentTarget.contains(e.relatedTarget as Node)) {
			isDragOverRef.current = false;
			setIsDragOver(false);
		}
	}, []);

	const handleDrop = useCallback(
		(e: React.DragEvent) => {
			e.preventDefault();
			isDragOverRef.current = false;
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
