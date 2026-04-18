import { invoke } from "@tauri-apps/api/core";
import { ArrowUp, Square, X } from "lucide-react";
import {
	useCallback,
	useImperativeHandle,
	useMemo,
	useRef,
	useState,
} from "react";
import { Button } from "@/components/ui/button";
import type { SlashCommand } from "@/hooks/useSlashCommands";
import { useSlashCommands } from "@/hooks/useSlashCommands";
import type {
	ImageAttachment,
	ModelInfo,
	PermissionMode,
} from "@/types/session";
import { ModelSelector } from "./ModelSelector";
import { ModeSelector } from "./ModeSelector";
import { SlashCommandPopup } from "./SlashCommandPopup";

interface AttachedImage {
	id: string;
	attachment: ImageAttachment;
	previewUrl: string;
}

export interface MessageInputHandle {
	addImageAttachments: (attachments: ImageAttachment[]) => void;
}

interface MessageInputProps {
	onSend: (content: string, images?: ImageAttachment[]) => void;
	onInterrupt: () => void;
	isStreaming: boolean;
	onCycleMode?: () => void;
	mode: PermissionMode;
	onModeChange: (mode: PermissionMode) => void;
	models: ModelInfo[];
	currentModelId: string | null;
	onModelChange: (modelId: string | null) => void;
	ref?: React.Ref<MessageInputHandle>;
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
	ref,
}: MessageInputProps) {
	const [value, setValue] = useState("");
	const textareaRef = useRef<HTMLTextAreaElement>(null);
	const [slashPopupDismissed, setSlashPopupDismissed] = useState(false);
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [attachedImages, setAttachedImages] = useState<AttachedImage[]>([]);
	const imageIdCounterRef = useRef(0);

	const allCommands = useSlashCommands();

	const createAttachedImage = useCallback(
		(attachment: ImageAttachment): AttachedImage => {
			return {
				id: `img-${++imageIdCounterRef.current}`,
				attachment,
				previewUrl: `data:${attachment.mediaType};base64,${attachment.data}`,
			};
		},
		[],
	);

	const processImageFile = useCallback(
		async (file: File): Promise<AttachedImage | null> => {
			const bytes = new Uint8Array(await file.arrayBuffer());
			try {
				const attachment = await invoke<ImageAttachment>(
					"prepare_image_attachment",
					{ data: Array.from(bytes) },
				);
				return createAttachedImage(attachment);
			} catch {
				return null;
			}
		},
		[createAttachedImage],
	);

	useImperativeHandle(
		ref,
		() => ({
			addImageAttachments: (attachments: ImageAttachment[]) => {
				const newImages = attachments.map(createAttachedImage);
				setAttachedImages((prev) => [...prev, ...newImages]);
			},
		}),
		[createAttachedImage],
	);

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

	const addImages = useCallback(
		async (files: File[]) => {
			const results = await Promise.all(files.map(processImageFile));
			const valid = results.filter((r): r is AttachedImage => r !== null);
			if (valid.length > 0) {
				setAttachedImages((prev) => [...prev, ...valid]);
			}
		},
		[processImageFile],
	);

	const removeImage = useCallback((id: string) => {
		setAttachedImages((prev) => prev.filter((img) => img.id !== id));
	}, []);

	const handleSubmit = useCallback(() => {
		const trimmed = value.trim();
		const hasImages = attachedImages.length > 0;
		if (!trimmed && !hasImages) return;
		if (hasImages) {
			onSend(
				trimmed,
				attachedImages.map((img) => img.attachment),
			);
		} else {
			onSend(trimmed);
		}
		setValue("");
		setAttachedImages([]);
		setSlashPopupDismissed(false);
		setSelectedIndex(0);
		if (textareaRef.current) {
			textareaRef.current.style.height = "auto";
		}
	}, [value, onSend, attachedImages]);

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

	const handlePaste = useCallback(
		(e: React.ClipboardEvent) => {
			const items = e.clipboardData?.items;
			if (!items) return;
			const imageFiles: File[] = [];
			for (const item of items) {
				if (item.type.startsWith("image/")) {
					const file = item.getAsFile();
					if (file) imageFiles.push(file);
				}
			}
			if (imageFiles.length > 0) {
				e.preventDefault();
				addImages(imageFiles);
			}
		},
		[addImages],
	);

	const canSend = value.trim().length > 0 || attachedImages.length > 0;

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
				{attachedImages.length > 0 && (
					<div
						data-testid="image-preview-list"
						className="flex flex-wrap gap-2 px-3 pt-2"
					>
						{attachedImages.map((img) => (
							<div
								key={img.id}
								className="relative group"
								data-testid="image-preview-item"
							>
								<img
									src={img.previewUrl}
									alt="Attached"
									className="h-16 w-16 object-cover rounded-md border"
								/>
								<button
									type="button"
									onClick={() => removeImage(img.id)}
									className="absolute -top-1.5 -right-1.5 bg-destructive text-destructive-foreground rounded-full p-0.5 opacity-0 group-hover:opacity-100 transition-opacity"
									aria-label="Remove image"
									data-testid="remove-image-button"
								>
									<X className="size-3" />
								</button>
							</div>
						))}
					</div>
				)}
				<textarea
					ref={textareaRef}
					value={value}
					onChange={handleChange}
					onKeyDown={handleKeyDown}
					onPaste={handlePaste}
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
