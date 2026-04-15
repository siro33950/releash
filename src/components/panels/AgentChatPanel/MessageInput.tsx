import { ArrowUp, Square } from "lucide-react";
import { useCallback, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import type { SlashCommand } from "@/hooks/useSlashCommands";
import { useSlashCommands } from "@/hooks/useSlashCommands";
import type { ModelInfo, PermissionMode } from "@/types/session";
import { ModelSelector } from "./ModelSelector";
import { ModeSelector } from "./ModeSelector";
import { SlashCommandPopup } from "./SlashCommandPopup";

interface MessageInputProps {
	onSend: (content: string) => void;
	onInterrupt: () => void;
	isStreaming: boolean;
	onCycleMode?: () => void;
	mode: PermissionMode;
	onModeChange: (mode: PermissionMode) => void;
	models: ModelInfo[];
	currentModelId: string | null;
	onModelChange: (modelId: string | null) => void;
}

export function MessageInput({
	onSend,
	onInterrupt,
	isStreaming,
	onCycleMode,
	mode,
	onModeChange,
	models,
	currentModelId,
	onModelChange,
}: MessageInputProps) {
	const [value, setValue] = useState("");
	const textareaRef = useRef<HTMLTextAreaElement>(null);
	const [slashPopupDismissed, setSlashPopupDismissed] = useState(false);
	const [selectedIndex, setSelectedIndex] = useState(0);

	const allCommands = useSlashCommands();

	const showSlashPopup =
		value.startsWith("/") && !value.includes(" ") && !slashPopupDismissed;
	const slashQuery = value.slice(1).toLowerCase();

	const filteredCommands = useMemo(() => {
		if (!showSlashPopup) return [];
		if (slashQuery === "") return allCommands;
		return allCommands.filter((cmd) =>
			cmd.name.toLowerCase().startsWith(slashQuery),
		);
	}, [showSlashPopup, slashQuery, allCommands]);

	const popupOpen = showSlashPopup && filteredCommands.length > 0;

	const handleSelectCommand = useCallback((cmd: SlashCommand) => {
		setValue(`/${cmd.name} `);
		setSlashPopupDismissed(true);
		setSelectedIndex(0);
		if (textareaRef.current) {
			textareaRef.current.focus();
		}
	}, []);

	const handleSubmit = useCallback(() => {
		const trimmed = value.trim();
		if (!trimmed) return;
		onSend(trimmed);
		setValue("");
		setSlashPopupDismissed(false);
		setSelectedIndex(0);
		if (textareaRef.current) {
			textareaRef.current.style.height = "auto";
		}
	}, [value, onSend]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent<HTMLTextAreaElement>) => {
			if (popupOpen) {
				if (e.key === "ArrowDown") {
					e.preventDefault();
					setSelectedIndex((i) =>
						i >= filteredCommands.length - 1 ? 0 : i + 1,
					);
					return;
				}
				if (e.key === "ArrowUp") {
					e.preventDefault();
					setSelectedIndex((i) =>
						i <= 0 ? filteredCommands.length - 1 : i - 1,
					);
					return;
				}
				if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
					e.preventDefault();
					if (filteredCommands[selectedIndex]) {
						handleSelectCommand(filteredCommands[selectedIndex]);
					}
					return;
				}
				if (e.key === "Tab" && !e.shiftKey) {
					e.preventDefault();
					if (filteredCommands[selectedIndex]) {
						handleSelectCommand(filteredCommands[selectedIndex]);
					}
					return;
				}
				if (e.key === "Escape") {
					e.preventDefault();
					setSlashPopupDismissed(true);
					return;
				}
			}

			if (e.key === "Tab" && e.shiftKey) {
				e.preventDefault();
				onCycleMode?.();
				return;
			}
			if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
				e.preventDefault();
				handleSubmit();
			}
		},
		[
			handleSubmit,
			onCycleMode,
			popupOpen,
			filteredCommands,
			selectedIndex,
			handleSelectCommand,
		],
	);

	const handleChange = useCallback(
		(e: React.ChangeEvent<HTMLTextAreaElement>) => {
			const newValue = e.target.value;
			setValue(newValue);
			setSlashPopupDismissed(false);
			setSelectedIndex(0);
			const el = e.target;
			el.style.height = "auto";
			el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
		},
		[],
	);

	const canSend = value.trim().length > 0;

	return (
		<SlashCommandPopup
			open={popupOpen}
			commands={filteredCommands}
			selectedIndex={selectedIndex}
			onSelect={handleSelectCommand}
			onClose={() => setSlashPopupDismissed(true)}
		>
			<div
				data-testid="message-input"
				className="mx-3 my-2 border rounded-lg focus-within:ring-1 focus-within:ring-ring"
			>
				<textarea
					ref={textareaRef}
					value={value}
					onChange={handleChange}
					onKeyDown={handleKeyDown}
					placeholder="Send a message..."
					rows={1}
					className="w-full resize-none bg-transparent border-none px-3 pt-3 pb-1 text-sm focus:outline-none min-h-[36px] max-h-[200px]"
				/>
				<div className="flex items-center justify-between px-2 pb-2">
					<div className="flex items-center gap-1">
						<ModeSelector
							mode={mode}
							onModeChange={onModeChange}
							disabled={false}
						/>
						<ModelSelector
							models={models}
							currentModelId={currentModelId}
							onModelChange={onModelChange}
							disabled={false}
						/>
					</div>
					{isStreaming && !canSend ? (
						<Button
							size="icon"
							variant="destructive"
							className="h-7 w-7 shrink-0"
							onClick={onInterrupt}
							aria-label="Interrupt agent"
						>
							<Square className="size-3.5" />
						</Button>
					) : (
						<Button
							size="icon"
							className="h-7 w-7 shrink-0"
							onClick={handleSubmit}
							disabled={!canSend}
							aria-label="Send message"
						>
							<ArrowUp className="size-3.5" />
						</Button>
					)}
				</div>
			</div>
		</SlashCommandPopup>
	);
}
