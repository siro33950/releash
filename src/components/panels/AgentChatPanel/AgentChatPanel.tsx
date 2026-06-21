import { invoke } from "@tauri-apps/api/core";
import type React from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	CommandDialog,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
	CommandShortcut,
} from "@/components/ui/command";
import { useAgentChatContext } from "@/contexts/AgentChatContext";
import { useDisplayedActiveSession } from "@/hooks/useDisplayedActiveSession";
import type { DropZoneType } from "@/hooks/useNativeFileDrop";
import type { AgentEditorSelection, MentionReference } from "@/types/session";
import type { CenterSelectionRequest } from "@/types/workspace-tree";
import { BoundSessionChat } from "./BoundSessionChat";

interface AgentCommandPaletteItem {
	id: string;
	label: string;
	shortcut: string;
	alternateShortcut?: string | null;
	enabled: boolean;
}

interface AgentShortcutSetting {
	id: string;
	label: string;
	shortcut: string;
	alternateShortcut?: string | null;
	defaultShortcut: string;
}

function normalizeKeyboardShortcut(event: KeyboardEvent): string | null {
	const key = normalizeKeyboardKey(event.key);
	if (!key) return null;
	const parts: string[] = [];
	if (event.metaKey) parts.push("Cmd");
	if (event.ctrlKey) parts.push("Ctrl");
	if (event.altKey) parts.push("Alt");
	if (event.shiftKey) parts.push("Shift");
	if (parts.length === 0) return null;
	parts.push(key);
	return parts.join(" ");
}

function normalizeKeyboardKey(key: string): string | null {
	if (
		!key ||
		key === "Meta" ||
		key === "Control" ||
		key === "Alt" ||
		key === "Shift"
	) {
		return null;
	}
	if (key === " ") return "Space";
	if (key.length === 1) return key.toUpperCase();
	return key.slice(0, 1).toUpperCase() + key.slice(1).toLowerCase();
}

function shortcutMatches(
	setting: AgentShortcutSetting,
	shortcut: string | null,
): boolean {
	if (!shortcut) return false;
	return (
		setting.shortcut === shortcut || setting.alternateShortcut === shortcut
	);
}

function commandShortcutLabel(item: AgentCommandPaletteItem): string {
	if (item.alternateShortcut) {
		return `${item.shortcut} / ${item.alternateShortcut}`;
	}
	return item.shortcut;
}

interface AgentChatPanelProps {
	worktreePath: string;
	selectionRequest?: CenterSelectionRequest | null;
	activeEditorPath?: string | null;
	openEditorPaths?: string[];
	activeEditorSelection?: AgentEditorSelection | null;
	registerDropZone: (
		zone: DropZoneType,
		element: HTMLElement | null,
		onDrop?: (paths: string[]) => void,
	) => void;
	sendMessageRef?: React.MutableRefObject<
		((content: string, mentions?: MentionReference[]) => Promise<void>) | null
	>;
	onOpenDiffFile?: (filePath: string) => void;
	onNewSessionCreated?: (sessionId: string) => void;
}

/**
 * 自由対話 chat の本文 panel。#1220 以降、Session 一覧・切替・close・history は
 * Workspace tree の責務であり、ここでは選択済み 1 session だけを表示する。
 */
export function AgentChatPanel({
	worktreePath,
	selectionRequest,
	activeEditorPath,
	openEditorPaths,
	activeEditorSelection,
	registerDropZone,
	sendMessageRef,
	onOpenDiffFile,
	onNewSessionCreated,
}: AgentChatPanelProps) {
	const { orderedSessions, selectSession, createNewSession } =
		useAgentChatContext();

	// spec issues-1023: workflow step として起動された chat session は
	// 自由対話 chat tab と同格に tab bar 上に並べない。観測経路は Workflow panel の
	// step conversation transcript 側に切り出されている。
	const displayedSessions = useMemo(
		() => orderedSessions.filter((s) => !s.workflowStepSession),
		[orderedSessions],
	);
	// spec issues-1023 / issues-1022: 万一 activeSession が workflow step session の状態でも
	// AgentChatPanel 本文では表示しない（Workflow panel 側 transcript の二重表示防止）。
	// Diff Thread handoff (issues-1022) と同じ判定規則を共通 hook 経由で参照する。
	const displayedActiveSession = useDisplayedActiveSession();
	const activeSessionId = displayedActiveSession?.id ?? null;

	const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
	const [commandPaletteItems, setCommandPaletteItems] = useState<
		AgentCommandPaletteItem[]
	>([]);
	const [agentShortcuts, setAgentShortcuts] = useState<AgentShortcutSetting[]>(
		[],
	);
	const handledSelectionRequestIdRef = useRef<number | null>(null);
	const createSessionAndRefreshTree = useCallback(async () => {
		const sessionId = await createNewSession();
		if (sessionId) {
			onNewSessionCreated?.(sessionId);
		}
		window.dispatchEvent(
			new CustomEvent("workspace-tree-refresh", {
				detail: { worktreePath },
			}),
		);
	}, [createNewSession, onNewSessionCreated, worktreePath]);

	useEffect(() => {
		if (!selectionRequest) return;
		if (selectionRequest.worktreePath !== worktreePath) return;
		if (handledSelectionRequestIdRef.current === selectionRequest.requestId) {
			return;
		}
		handledSelectionRequestIdRef.current = selectionRequest.requestId;

		if (selectionRequest.kind === "agentSession") {
			void selectSession(selectionRequest.sessionId);
		} else if (selectionRequest.kind === "newAgentSession") {
			void createSessionAndRefreshTree();
		}
	}, [
		createSessionAndRefreshTree,
		selectSession,
		selectionRequest,
		worktreePath,
	]);

	useEffect(() => {
		let cancelled = false;
		void invoke<AgentShortcutSetting[]>("get_agent_shortcut_settings")
			.then((settings) => {
				if (cancelled) return;
				setAgentShortcuts(settings);
			})
			.catch(() => {
				if (cancelled) return;
				setAgentShortcuts([]);
			});
		return () => {
			cancelled = true;
		};
	}, []);

	useEffect(() => {
		if (!commandPaletteOpen) return;
		let cancelled = false;
		void invoke<AgentCommandPaletteItem[]>("present_agent_command_palette", {
			request: {
				hasActiveSession: Boolean(activeSessionId),
				sessionCount: displayedSessions.length,
			},
		})
			.then((items) => {
				if (cancelled) return;
				setCommandPaletteItems(items);
			})
			.catch(() => {
				if (cancelled) return;
				setCommandPaletteItems([]);
			});
		return () => {
			cancelled = true;
		};
	}, [activeSessionId, commandPaletteOpen, displayedSessions.length]);

	const selectAdjacentSession = useCallback(
		(direction: -1 | 1) => {
			if (!activeSessionId || displayedSessions.length < 2) return;
			const currentIndex = displayedSessions.findIndex(
				(session) => session.id === activeSessionId,
			);
			if (currentIndex === -1) return;
			const nextIndex =
				(currentIndex + direction + displayedSessions.length) %
				displayedSessions.length;
			selectSession(displayedSessions[nextIndex].id);
		},
		[activeSessionId, displayedSessions, selectSession],
	);

	const runAgentCommand = useCallback(
		(commandId: string) => {
			setCommandPaletteOpen(false);
			switch (commandId) {
				case "command_menu":
					setCommandPaletteOpen(true);
					return;
				case "new_thread":
					void createSessionAndRefreshTree();
					return;
				case "search_threads":
					window.dispatchEvent(new Event("agent-open-thread-history"));
					return;
				case "find_in_thread":
					window.dispatchEvent(new Event("agent-open-thread-find"));
					return;
				case "copy_latest_response":
					window.dispatchEvent(new Event("agent-copy-latest-response"));
					return;
				case "toggle_raw_scrollback":
					window.dispatchEvent(new Event("agent-toggle-raw-scrollback"));
					return;
				case "previous_thread":
					selectAdjacentSession(-1);
					return;
				case "next_thread":
					selectAdjacentSession(1);
					return;
			}
		},
		[createSessionAndRefreshTree, selectAdjacentSession],
	);

	const runCommandPaletteItem = useCallback(
		(item: AgentCommandPaletteItem) => {
			if (!item.enabled) return;
			runAgentCommand(item.id);
		},
		[runAgentCommand],
	);

	useEffect(() => {
		const handleKeyDown = (event: KeyboardEvent) => {
			const shortcut = normalizeKeyboardShortcut(event);
			const matched = agentShortcuts.find((setting) =>
				shortcutMatches(setting, shortcut),
			);
			if (!matched) return;
			event.preventDefault();
			if (matched.id === "command_menu") {
				runAgentCommand(matched.id);
				return;
			}
			void invoke<boolean>("is_agent_command_enabled", {
				request: {
					commandId: matched.id,
					request: {
						hasActiveSession: Boolean(activeSessionId),
						sessionCount: displayedSessions.length,
					},
				},
			})
				.then((enabled) => {
					if (enabled !== true) return;
					runAgentCommand(matched.id);
				})
				.catch(() => {});
		};
		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [
		activeSessionId,
		agentShortcuts,
		displayedSessions.length,
		runAgentCommand,
	]);

	return (
		<div data-testid="agent-chat-panel" className="flex flex-col h-full">
			{activeSessionId ? (
				<BoundSessionChat
					sessionId={activeSessionId}
					worktreePath={worktreePath}
					activeEditorPath={activeEditorPath}
					openEditorPaths={openEditorPaths}
					activeEditorSelection={activeEditorSelection}
					registerDropZone={registerDropZone}
					dropZoneName="agent"
					sendMessageRef={sendMessageRef}
					onOpenDiffFile={onOpenDiffFile}
					skipInitialLoad
				/>
			) : (
				<div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
					No chat selected
				</div>
			)}
			<CommandDialog
				open={commandPaletteOpen}
				onOpenChange={setCommandPaletteOpen}
				title="Agent Commands"
				description="Search agent commands"
				className="max-w-lg"
			>
				<CommandInput placeholder="Search commands" />
				<CommandList data-testid="agent-command-palette">
					<CommandEmpty>No commands found</CommandEmpty>
					<CommandGroup heading="Agent">
						{commandPaletteItems.map((item) => (
							<CommandItem
								key={item.id}
								value={item.label}
								disabled={!item.enabled}
								onSelect={() => runCommandPaletteItem(item)}
							>
								<span>{item.label}</span>
								<CommandShortcut>{commandShortcutLabel(item)}</CommandShortcut>
							</CommandItem>
						))}
					</CommandGroup>
				</CommandList>
			</CommandDialog>
		</div>
	);
}
