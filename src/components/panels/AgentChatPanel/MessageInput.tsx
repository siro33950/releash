import { invoke } from "@tauri-apps/api/core";
import { ArrowUp, Square, X } from "lucide-react";
import {
	useCallback,
	useEffect,
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
	MentionReference,
	ModelInfo,
	PermissionMode,
} from "@/types/session";
import { MentionPopup } from "./MentionPopup";
import { ModelSelector } from "./ModelSelector";

const MENTION_SYNC_RE =
	/@([^ \t\r\n@:]+(?:\.[^ \t\r\n@:]+)*)(?::L(\d+)(?:-L(\d+))?)?/g;

function syncMentionsWithText(
	text: string,
	refs: MentionReference[],
): MentionReference[] | undefined {
	const re = new RegExp(MENTION_SYNC_RE.source, "g");
	const found = new Map<string, { startLine?: number; endLine?: number }>();
	for (;;) {
		const m = re.exec(text);
		if (m === null) break;
		found.set(m[1], {
			startLine: m[2] ? Number(m[2]) : undefined,
			endLine: m[3] ? Number(m[3]) : undefined,
		});
	}
	const synced = refs
		.filter((ref) => found.has(ref.filePath))
		.map((ref) => {
			const info = found.get(ref.filePath);
			if (!info) return { filePath: ref.filePath };
			return {
				filePath: ref.filePath,
				startLine: info.startLine,
				endLine: info.endLine,
			};
		});
	return synced.length > 0 ? synced : undefined;
}

import { ModeSelector } from "./ModeSelector";
import { findMentionTrigger, handlePopupKeyDown } from "./popupInputUtils";
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
	onSend: (
		content: string,
		images?: ImageAttachment[],
		mentions?: MentionReference[],
	) => void;
	onInterrupt: () => void;
	isStreaming: boolean;
	onCycleMode?: () => void;
	mode: PermissionMode;
	onModeChange: (mode: PermissionMode) => void;
	models: ModelInfo[];
	currentModelId: string | null;
	onModelChange: (modelId: string | null) => void;
	ref?: React.Ref<MessageInputHandle>;
	worktreePath?: string;
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
	worktreePath,
}: MessageInputProps) {
	const [value, setValue] = useState("");
	const textareaRef = useRef<HTMLTextAreaElement>(null);
	const [slashPopupDismissed, setSlashPopupDismissed] = useState(false);
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [attachedImages, setAttachedImages] = useState<AttachedImage[]>([]);
	const imageIdCounterRef = useRef(0);

	// Mention state
	const [mentionDismissed, setMentionDismissed] = useState(false);
	const [mentionSelectedIndex, setMentionSelectedIndex] = useState(0);
	const [mentionFiles, setMentionFiles] = useState<string[]>([]);
	const [mentionTrigger, setMentionTrigger] = useState<{
		start: number;
		query: string;
	} | null>(null);
	const [mentionRefs, setMentionRefs] = useState<MentionReference[]>([]);

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
	const mentionPopupOpen =
		!mentionDismissed && mentionTrigger !== null && mentionFiles.length > 0;
	const mentionQuery = mentionTrigger?.query ?? null;

	// Fetch mention candidates when trigger changes (debounced)
	useEffect(() => {
		if (mentionQuery === null || mentionDismissed || !worktreePath) {
			setMentionFiles([]);
			return;
		}

		setMentionFiles([]);
		setMentionSelectedIndex(0);

		let cancelled = false;
		const timer = setTimeout(() => {
			invoke<string[]>("list_mentionable_files", {
				worktreePath,
				query: mentionQuery,
			})
				.then((files) => {
					if (!cancelled) {
						setMentionFiles(files);
						setMentionSelectedIndex(0);
					}
				})
				.catch(() => {
					if (!cancelled) {
						setMentionFiles([]);
					}
				});
		}, 150);

		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [mentionQuery, mentionDismissed, worktreePath]);

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

	const handleSelectMention = useCallback(
		(filePath: string) => {
			if (!mentionTrigger) return;
			// Replace @query with @filePath
			const before = value.slice(0, mentionTrigger.start);
			const after = value
				.slice(mentionTrigger.start + 1 + mentionTrigger.query.length)
				.replace(/^\s/, "");
			const newValue = `${before}@${filePath} ${after}`;
			setValue(newValue);
			setMentionRefs((prev) => [
				...prev,
				{ filePath, startLine: undefined, endLine: undefined },
			]);
			setMentionTrigger(null);
			setMentionDismissed(true);
			setMentionSelectedIndex(0);
			const caret = before.length + 1 + filePath.length + 1; // "@" + path + " "
			const el = textareaRef.current;
			if (el) {
				el.focus();
				requestAnimationFrame(() => {
					el.setSelectionRange(caret, caret);
				});
			}
		},
		[mentionTrigger, value],
	);

	const handleSubmit = useCallback(() => {
		const trimmed = value.trim();
		const hasImages = attachedImages.length > 0;
		if (!trimmed && !hasImages) return;
		const currentMentions = syncMentionsWithText(trimmed, mentionRefs);
		if (hasImages) {
			onSend(
				trimmed,
				attachedImages.map((img) => img.attachment),
				currentMentions,
			);
		} else {
			onSend(trimmed, undefined, currentMentions);
		}
		setValue("");
		setAttachedImages([]);
		setMentionRefs([]);
		setSlashPopupDismissed(false);
		setSelectedIndex(0);
		setMentionTrigger(null);
		setMentionDismissed(false);
		setMentionSelectedIndex(0);
		if (textareaRef.current) {
			textareaRef.current.style.height = "auto";
		}
	}, [value, onSend, attachedImages, mentionRefs]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent<HTMLTextAreaElement>) => {
			// Mention popup keyboard handling (takes priority when open)
			if (mentionPopupOpen) {
				if (
					handlePopupKeyDown(
						e,
						mentionFiles.length,
						setMentionSelectedIndex,
						() => {
							if (mentionFiles[mentionSelectedIndex]) {
								handleSelectMention(mentionFiles[mentionSelectedIndex]);
							}
						},
						() => setMentionDismissed(true),
					)
				)
					return;
			}

			// Slash command popup keyboard handling
			if (popupOpen) {
				if (
					handlePopupKeyDown(
						e,
						filteredCommands.length,
						setSelectedIndex,
						() => {
							if (filteredCommands[selectedIndex]) {
								handleSelectCommand(filteredCommands[selectedIndex]);
							}
						},
						() => setSlashPopupDismissed(true),
					)
				)
					return;
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
			mentionPopupOpen,
			mentionFiles,
			mentionSelectedIndex,
			handleSelectMention,
		],
	);

	const handleChange = useCallback(
		(e: React.ChangeEvent<HTMLTextAreaElement>) => {
			const newValue = e.target.value;
			setValue(newValue);
			setSlashPopupDismissed(false);
			setSelectedIndex(0);

			// Detect mention trigger
			const cursorPos = e.target.selectionStart ?? newValue.length;
			const trigger = findMentionTrigger(newValue, cursorPos);
			if (trigger) {
				setMentionTrigger(trigger);
				setMentionDismissed(false);
			} else {
				setMentionTrigger(null);
			}

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
	const inputRef = useRef<HTMLDivElement>(null);

	return (
		<>
			<div
				ref={inputRef}
				data-testid="message-input"
				className="relative mx-3 my-2 border rounded-lg focus-within:ring-1 focus-within:ring-ring"
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
									className="absolute -top-1.5 -right-1.5 bg-destructive text-destructive-foreground rounded-full p-0.5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus:opacity-100 focus:outline-none focus:ring-2 focus:ring-ring transition-opacity"
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
			<SlashCommandPopup
				open={popupOpen}
				commands={filteredCommands}
				selectedIndex={selectedIndex}
				onSelect={handleSelectCommand}
				onClose={() => setSlashPopupDismissed(true)}
				anchorRef={inputRef}
			/>
			<MentionPopup
				open={mentionPopupOpen}
				files={mentionFiles}
				selectedIndex={mentionSelectedIndex}
				onSelect={handleSelectMention}
				onClose={() => setMentionDismissed(true)}
				anchorRef={inputRef}
			/>
		</>
	);
}
