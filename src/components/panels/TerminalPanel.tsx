import { invoke } from "@tauri-apps/api/core";
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
	ContextMenuShortcut,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import type { NativeFileDropPayload } from "@/hooks/useNativeFileDrop";
import {
	type TerminalInitializationMode,
	type TerminalSurfaceOwner,
	useTerminal,
} from "@/hooks/useTerminal";
import type { Theme } from "@/types/settings";
import "@xterm/xterm/css/xterm.css";

export interface TerminalPanelHandle {
	writeToTerminal: (data: string) => void;
	requestKill: () => void;
}

export interface TerminalPanelProps {
	cwd?: string | null;
	theme?: Theme;
	terminalStartupCommand?: string;
	owner?: TerminalSurfaceOwner;
	label?: string;
	onTerminalReady?: (sessionKey: string) => void;
	onTerminalError?: (message: string) => void;
	shouldKillPendingTerminal?: () => boolean;
	initialization?: TerminalInitializationMode;
	autoFocus?: boolean;
	onSplitVertical?: () => void;
	onSplitHorizontal?: () => void;
	onBreakToTab?: () => void;
	onClosePane?: () => void;
	canBreakToTab?: boolean;
	isOnlyPane?: boolean;
}

export const TerminalPanel = forwardRef<
	TerminalPanelHandle,
	TerminalPanelProps
>(function TerminalPanel(
	{
		cwd,
		theme,
		terminalStartupCommand,
		owner,
		label,
		onTerminalReady,
		onTerminalError,
		shouldKillPendingTerminal,
		initialization,
		autoFocus,
		onSplitVertical,
		onSplitHorizontal,
		onBreakToTab,
		onClosePane,
		canBreakToTab,
		isOnlyPane,
	},
	ref,
) {
	const containerRef = useRef<HTMLDivElement>(null);
	const { terminalRef, terminalOwner, isRunningRef, sendInput, requestKill } =
		useTerminal(containerRef, {
			cwd,
			theme,
			terminalStartupCommand,
			owner,
			label,
			onTerminalReady,
			onTerminalError,
			shouldKillPendingTerminal,
			initialization,
			autoFocus,
		});
	const [isDragOver, setIsDragOver] = useState(false);
	const isDragOverRef = useRef(false);

	const writePathsToTerminal = useCallback(
		(paths: string[]) => {
			if (!isRunningRef.current || paths.length === 0) return;
			invoke("write_paths_to_terminal_surface", {
				owner: terminalOwner,
				paths,
			}).catch((error) => {
				console.error("Failed to write paths to PTY:", error);
			});
		},
		[isRunningRef, terminalOwner],
	);

	useImperativeHandle(
		ref,
		() => ({
			writeToTerminal: sendInput,
			requestKill,
		}),
		[sendInput, requestKill],
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
						writePathsToTerminal(paths);
					}
				}
				isDragOverRef.current = false;
				setIsDragOver(false);
			},
		);
		return () => {
			unlisten.then((f) => f());
		};
	}, [writePathsToTerminal]);

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
				writePathsToTerminal([filePath]);
			}
		},
		[writePathsToTerminal],
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
					className="relative h-full w-full bg-terminal-bg p-2"
					onDragOver={handleDragOver}
					onDragLeave={handleDragLeave}
					onDrop={handleDrop}
				>
					<div ref={containerRef} className="h-full w-full bg-terminal-bg" />
					{isDragOver && (
						<div className="absolute inset-0 flex items-center justify-center bg-primary/10 border-2 border-dashed border-primary rounded pointer-events-none">
							<span className="text-sm font-medium text-primary bg-background/80 px-3 py-1.5 rounded">
								Drop to insert path
							</span>
						</div>
					)}
				</div>
			</ContextMenuTrigger>
			<ContextMenuContent className="w-56">
				<ContextMenuItem onClick={handleCopy}>Copy</ContextMenuItem>
				<ContextMenuItem onClick={handlePaste}>Paste</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onClick={handleSelectAll}>Select All</ContextMenuItem>
				<ContextMenuItem onClick={handleClear}>Clear</ContextMenuItem>
				<ContextMenuSeparator />
				{onSplitVertical && (
					<ContextMenuItem onClick={onSplitVertical}>
						Split Vertical
						<ContextMenuShortcut>⌘D</ContextMenuShortcut>
					</ContextMenuItem>
				)}
				{onSplitHorizontal && (
					<ContextMenuItem onClick={onSplitHorizontal}>
						Split Horizontal
						<ContextMenuShortcut>⇧⌘D</ContextMenuShortcut>
					</ContextMenuItem>
				)}
				{canBreakToTab && onBreakToTab && (
					<>
						<ContextMenuSeparator />
						<ContextMenuItem onClick={onBreakToTab}>
							Move to Tab
							<ContextMenuShortcut>⇧⌘T</ContextMenuShortcut>
						</ContextMenuItem>
					</>
				)}
				{!isOnlyPane && onClosePane && (
					<>
						<ContextMenuSeparator />
						<ContextMenuItem variant="destructive" onClick={onClosePane}>
							Close
						</ContextMenuItem>
					</>
				)}
			</ContextMenuContent>
		</ContextMenu>
	);
});
