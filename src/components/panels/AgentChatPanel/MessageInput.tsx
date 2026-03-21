import { ArrowUp, Square } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import type { PermissionMode } from "@/types/session";
import { ModeSelector } from "./ModeSelector";

interface MessageInputProps {
	onSend: (content: string) => void;
	onInterrupt: () => void;
	disabled: boolean;
	isStreaming: boolean;
	onCycleMode?: () => void;
	mode: PermissionMode;
	onModeChange: (mode: PermissionMode) => void;
}

export function MessageInput({
	onSend,
	onInterrupt,
	disabled,
	isStreaming,
	onCycleMode,
	mode,
	onModeChange,
}: MessageInputProps) {
	const [value, setValue] = useState("");
	const textareaRef = useRef<HTMLTextAreaElement>(null);

	const handleSubmit = useCallback(() => {
		const trimmed = value.trim();
		if (!trimmed || disabled) return;
		onSend(trimmed);
		setValue("");
		if (textareaRef.current) {
			textareaRef.current.style.height = "auto";
		}
	}, [value, disabled, onSend]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent<HTMLTextAreaElement>) => {
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
		[handleSubmit, onCycleMode],
	);

	const handleChange = useCallback(
		(e: React.ChangeEvent<HTMLTextAreaElement>) => {
			setValue(e.target.value);
			const el = e.target;
			el.style.height = "auto";
			el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
		},
		[],
	);

	const canSend = value.trim().length > 0 && !disabled;

	return (
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
				disabled={disabled}
				rows={1}
				className="w-full resize-none bg-transparent border-none px-3 pt-3 pb-1 text-sm focus:outline-none disabled:opacity-50 min-h-[36px] max-h-[200px]"
			/>
			<div className="flex items-center justify-between px-2 pb-2">
				<ModeSelector
					mode={mode}
					onModeChange={onModeChange}
					disabled={false}
				/>
				{isStreaming ? (
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
	);
}
