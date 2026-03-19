import { ArrowUp, Square } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { Button } from "@/components/ui/button";

interface MessageInputProps {
	onSend: (content: string) => void;
	onInterrupt: () => void;
	disabled: boolean;
	isStreaming: boolean;
}

export function MessageInput({
	onSend,
	onInterrupt,
	disabled,
	isStreaming,
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
			if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
				e.preventDefault();
				handleSubmit();
			}
		},
		[handleSubmit],
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
			className="border-t border-border p-3 flex gap-2 items-end"
		>
			<textarea
				ref={textareaRef}
				value={value}
				onChange={handleChange}
				onKeyDown={handleKeyDown}
				placeholder="Send a message..."
				disabled={disabled}
				rows={1}
				className="flex-1 resize-none bg-muted rounded-lg px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-ring disabled:opacity-50 min-h-[36px] max-h-[200px]"
			/>
			{isStreaming ? (
				<Button
					size="icon"
					variant="destructive"
					className="h-9 w-9 shrink-0"
					onClick={onInterrupt}
					aria-label="Interrupt agent"
				>
					<Square className="size-4" />
				</Button>
			) : (
				<Button
					size="icon"
					className="h-9 w-9 shrink-0"
					onClick={handleSubmit}
					disabled={!canSend}
					aria-label="Send message"
				>
					<ArrowUp className="size-4" />
				</Button>
			)}
		</div>
	);
}
