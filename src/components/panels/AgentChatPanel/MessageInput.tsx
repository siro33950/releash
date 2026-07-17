import { invoke } from "@tauri-apps/api/core";
import { ArrowUp, Loader2, Square, X } from "lucide-react";
import {
	useCallback,
	useEffect,
	useId,
	useImperativeHandle,
	useMemo,
	useRef,
	useState,
} from "react";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import type {
	AgentSkill,
	BackendInfo,
	ImageAttachment,
	MentionReference,
	ModelInfo,
	PermissionMode,
	PlanMode,
	SlashCommand,
} from "@/types/session";
import { MentionPopup } from "./MentionPopup";
import { ModelSelector } from "./ModelSelector";
import { ModeSelector } from "./ModeSelector";
import {
	findMentionTrigger,
	findSkillTrigger,
	handlePopupKeyDown,
} from "./popupInputUtils";
import { SkillPopup } from "./SkillPopup";
import { SlashCommandPopup } from "./SlashCommandPopup";

function mentionTokenText(filePath: string): string {
	if (!/[\s"]/.test(filePath)) return `@${filePath}`;
	return `@"${filePath.replace(/["\\]/g, "\\$&")}"`;
}

interface AttachedImage {
	id: string;
	attachment: ImageAttachment;
	previewUrl: string;
}

interface PastedTextBlock {
	id: number;
	placeholder: string;
	content: string;
}

export interface MessageInputHandle {
	addImageAttachments: (attachments: ImageAttachment[]) => void;
}

interface MessageInputProps {
	onSend: (
		content: string,
		images?: ImageAttachment[],
		mentions?: MentionReference[],
	) => Promise<boolean>;
	onInterrupt: () => void;
	isStreaming: boolean;
	/** interrupt 要求済みで turn 終了待ちの楽観状態。停止ボタンを停止中表示にする。*/
	isInterrupting?: boolean;
	onCycleMode?: () => void;
	mode: PermissionMode;
	onModeChange: (mode: PermissionMode) => void;
	planMode: PlanMode;
	onPlanModeChange: (enabled: PlanMode) => void;
	models: ModelInfo[];
	currentModelId: string;
	onModelChange: (modelId: string) => void;
	currentBackendId: string | null;
	canChangeBackend?: boolean;
	backends?: BackendInfo[];
	onBackendChange?: (backendId: string) => void;
	backendDisabled?: boolean;
	ref?: React.Ref<MessageInputHandle>;
	worktreePath?: string;
	promptSuggestion?: string | null;
	runtimeSlashCommands?: SlashCommand[];
}

export function MessageInput({
	onSend,
	onInterrupt,
	isStreaming,
	isInterrupting = false,
	onCycleMode,
	mode,
	onModeChange,
	planMode,
	onPlanModeChange,
	models,
	currentModelId,
	onModelChange,
	currentBackendId,
	canChangeBackend = true,
	backends = [],
	ref,
	worktreePath,
	promptSuggestion,
	runtimeSlashCommands = [],
}: MessageInputProps) {
	const planModeSwitchId = useId();
	const [value, setValue] = useState("");
	const draftRevisionRef = useRef(0);
	const textareaRef = useRef<HTMLTextAreaElement>(null);
	const [isSubmitting, setIsSubmitting] = useState(false);
	const isSubmittingRef = useRef(false);
	const [slashPopupDismissed, setSlashPopupDismissed] = useState(false);
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [attachedImages, setAttachedImages] = useState<AttachedImage[]>([]);
	const imageIdCounterRef = useRef(0);
	const pastedTextIdCounterRef = useRef(0);
	const [pastedTextBlocks, setPastedTextBlocks] = useState<PastedTextBlock[]>(
		[],
	);
	// Mention state
	const [mentionDismissed, setMentionDismissed] = useState(false);
	const [mentionSelectedIndex, setMentionSelectedIndex] = useState(0);
	const [mentionFiles, setMentionFiles] = useState<string[]>([]);
	const [mentionTrigger, setMentionTrigger] = useState<{
		start: number;
		query: string;
	} | null>(null);
	const [mentionRefs, setMentionRefs] = useState<MentionReference[]>([]);
	const [skillDismissed, setSkillDismissed] = useState(false);
	const [skillSelectedIndex, setSkillSelectedIndex] = useState(0);
	const [skillCandidates, setSkillCandidates] = useState<AgentSkill[]>([]);
	const [skillTrigger, setSkillTrigger] = useState<{
		start: number;
		query: string;
	} | null>(null);

	const allCommands = useMemo(() => {
		const seen = new Set<string>();
		const merged: SlashCommand[] = [];
		for (const command of runtimeSlashCommands) {
			if (seen.has(command.name)) continue;
			seen.add(command.name);
			merged.push(command);
		}
		return merged;
	}, [runtimeSlashCommands]);
	const setComposerValue = useCallback((nextValue: string) => {
		draftRevisionRef.current += 1;
		setValue(nextValue);
		requestAnimationFrame(() => {
			const el = textareaRef.current;
			if (!el) return;
			el.style.height = "auto";
			el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
			el.setSelectionRange(nextValue.length, nextValue.length);
		});
	}, []);
	const setComposerValueWithCaret = useCallback(
		(nextValue: string, caret: number) => {
			draftRevisionRef.current += 1;
			setValue(nextValue);
			requestAnimationFrame(() => {
				const el = textareaRef.current;
				if (!el) return;
				el.style.height = "auto";
				el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
				el.setSelectionRange(caret, caret);
			});
		},
		[],
	);

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
	const skillPopupOpen =
		!skillDismissed && skillTrigger !== null && skillCandidates.length > 0;
	const skillQuery = skillTrigger?.query ?? null;
	const activePromptSuggestion =
		value.trim().length === 0 && attachedImages.length === 0
			? promptSuggestion?.trim() || null
			: null;
	const supportsActiveTurnSteering =
		backends.find((backend) => backend.id === currentBackendId)?.capabilities
			?.steering ?? false;
	const activeSlashArgumentHelp = useMemo(() => {
		const match = value.match(/^\/([^\s]+)\s*$/);
		if (!match || !value.endsWith(" ")) return null;
		const command = allCommands.find((cmd) => cmd.name === match[1]);
		if (!command?.argumentHint) return null;
		return command;
	}, [allCommands, value]);

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
				backendId: currentBackendId ?? undefined,
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
	}, [mentionQuery, mentionDismissed, worktreePath, currentBackendId]);

	useEffect(() => {
		if (skillQuery === null || skillDismissed || !worktreePath) {
			setSkillCandidates([]);
			return;
		}

		setSkillCandidates([]);
		setSkillSelectedIndex(0);

		let cancelled = false;
		const normalizedQuery = skillQuery.trim().toLowerCase();
		const timer = setTimeout(() => {
			invoke<AgentSkill[]>("scan_agent_skills", {
				cwd: worktreePath,
				query: normalizedQuery,
				limit: 20,
				backendId: currentBackendId ?? undefined,
			})
				.then((skills) => {
					if (cancelled) return;
					setSkillCandidates(skills);
					setSkillSelectedIndex(0);
				})
				.catch(() => {
					if (!cancelled) {
						setSkillCandidates([]);
					}
				});
		}, 150);

		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [skillQuery, skillDismissed, worktreePath, currentBackendId]);

	const handleSelectCommand = useCallback((cmd: SlashCommand) => {
		const nextValue = `/${cmd.name} `;
		draftRevisionRef.current += 1;
		setValue(nextValue);
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

	const expandSubmittedContent = useCallback(
		async (content: string): Promise<string> => {
			if (pastedTextBlocks.length === 0) return content;
			return invoke<string>("expand_pasted_text_blocks", {
				content,
				blocks: pastedTextBlocks,
			});
		},
		[pastedTextBlocks],
	);
	const syncMentionsForSubmit = useCallback(
		async (content: string): Promise<MentionReference[] | undefined> => {
			if (mentionRefs.length === 0) return undefined;
			return invoke<MentionReference[] | null>("sync_mentions_with_text", {
				text: content,
				refs: mentionRefs,
			}).then((mentions) => mentions ?? undefined);
		},
		[mentionRefs],
	);

	const handleSelectMention = useCallback(
		(filePath: string) => {
			if (!mentionTrigger) return;
			const token = mentionTokenText(filePath);
			const before = value.slice(0, mentionTrigger.start);
			const replacementEnd =
				textareaRef.current?.selectionStart ??
				mentionTrigger.start + 1 + mentionTrigger.query.length;
			const after = value.slice(replacementEnd).replace(/^\s/, "");
			const newValue = `${before}${token} ${after}`;
			draftRevisionRef.current += 1;
			setValue(newValue);
			setMentionRefs((prev) => [
				...prev,
				{ filePath, startLine: undefined, endLine: undefined },
			]);
			setMentionTrigger(null);
			setMentionDismissed(true);
			setMentionSelectedIndex(0);
			const caret = before.length + token.length + 1;
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

	const handleSelectSkill = useCallback(
		(skill: AgentSkill) => {
			if (!skillTrigger) return;
			const before = value.slice(0, skillTrigger.start);
			const after = value
				.slice(skillTrigger.start + 1 + skillTrigger.query.length)
				.replace(/^\s/, "");
			const token = `/${skill.name}`;
			const newValue = `${before}${token} ${after}`;
			draftRevisionRef.current += 1;
			setValue(newValue);
			setSkillTrigger(null);
			setSkillDismissed(true);
			setSkillSelectedIndex(0);
			const caret = before.length + token.length + 1;
			const el = textareaRef.current;
			if (el) {
				el.focus();
				requestAnimationFrame(() => {
					el.setSelectionRange(caret, caret);
				});
			}
		},
		[skillTrigger, value],
	);

	const handleSubmit = useCallback(() => {
		if (isSubmittingRef.current) return;
		const trimmed = value.trim();
		const hasImages = attachedImages.length > 0;
		if (!trimmed && !hasImages) return;

		const submittedDraftRevision = draftRevisionRef.current;
		const submittedImages = attachedImages;
		const submittedImageIds = new Set(submittedImages.map((image) => image.id));
		const submittedPastedTextIds = new Set(
			pastedTextBlocks.map((block) => block.id),
		);
		const submittedMentions = mentionRefs;
		isSubmittingRef.current = true;
		setIsSubmitting(true);

		const submitContent = async () => {
			let failureStage: "pre-send processing" | "send" = "pre-send processing";
			try {
				let submittedContent = trimmed;
				if (pastedTextBlocks.length > 0) {
					try {
						submittedContent = await expandSubmittedContent(trimmed);
					} catch (error) {
						console.error("Failed to expand pasted text blocks:", error);
						return;
					}
				}
				const currentMentions =
					submittedMentions.length === 0
						? undefined
						: await syncMentionsForSubmit(submittedContent);
				failureStage = "send";
				const sent = await onSend(
					submittedContent,
					hasImages
						? submittedImages.map((image) => image.attachment)
						: undefined,
					currentMentions,
				);
				if (sent !== true) return;

				const draftWasEdited =
					draftRevisionRef.current !== submittedDraftRevision;
				if (!draftWasEdited) {
					setValue("");
					setPastedTextBlocks((current) =>
						current.filter((block) => !submittedPastedTextIds.has(block.id)),
					);
					setMentionRefs((current) => {
						const remainingSubmitted = [...submittedMentions];
						return current.filter((mention) => {
							const submittedIndex = remainingSubmitted.findIndex(
								(submitted) =>
									submitted.filePath === mention.filePath &&
									submitted.startLine === mention.startLine &&
									submitted.endLine === mention.endLine,
							);
							if (submittedIndex === -1) return true;
							remainingSubmitted.splice(submittedIndex, 1);
							return false;
						});
					});
				}
				setAttachedImages((current) =>
					current.filter((image) => !submittedImageIds.has(image.id)),
				);
				if (!draftWasEdited) {
					setSlashPopupDismissed(false);
					setSelectedIndex(0);
					setMentionTrigger(null);
					setMentionDismissed(false);
					setMentionSelectedIndex(0);
					setSkillTrigger(null);
					setSkillDismissed(false);
					setSkillSelectedIndex(0);
					if (textareaRef.current) {
						textareaRef.current.style.height = "auto";
					}
				}
			} catch (error) {
				console.error(`Message ${failureStage} failed:`, error);
			} finally {
				isSubmittingRef.current = false;
				setIsSubmitting(false);
			}
		};

		void submitContent();
	}, [
		value,
		onSend,
		attachedImages,
		expandSubmittedContent,
		mentionRefs,
		syncMentionsForSubmit,
		pastedTextBlocks,
	]);

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

			if (skillPopupOpen) {
				if (
					handlePopupKeyDown(
						e,
						skillCandidates.length,
						setSkillSelectedIndex,
						() => {
							if (skillCandidates[skillSelectedIndex]) {
								handleSelectSkill(skillCandidates[skillSelectedIndex]);
							}
						},
						() => setSkillDismissed(true),
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
			if (
				activePromptSuggestion &&
				(e.key === "Tab" || e.key === "ArrowRight") &&
				!e.shiftKey &&
				!e.metaKey &&
				!e.ctrlKey &&
				!e.altKey
			) {
				e.preventDefault();
				setComposerValue(activePromptSuggestion);
				return;
			}
			if (
				isStreaming &&
				value.length === 0 &&
				e.key.toLowerCase() === "c" &&
				e.ctrlKey &&
				!e.metaKey &&
				!e.altKey
			) {
				e.preventDefault();
				onInterrupt();
				return;
			}
			if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
				e.preventDefault();
				handleSubmit();
				return;
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
			skillPopupOpen,
			skillCandidates,
			skillSelectedIndex,
			handleSelectSkill,
			activePromptSuggestion,
			isStreaming,
			onInterrupt,
			value,
			setComposerValue,
		],
	);

	const handleChange = useCallback(
		(e: React.ChangeEvent<HTMLTextAreaElement>) => {
			const newValue = e.target.value;
			draftRevisionRef.current += 1;
			setValue(newValue);
			setSlashPopupDismissed(false);
			setSelectedIndex(0);

			// Detect mention trigger
			const cursorPos = e.target.selectionStart ?? newValue.length;
			const trigger = findMentionTrigger(newValue, cursorPos);
			if (trigger) {
				setMentionTrigger(trigger);
				setMentionDismissed(false);
				setSkillTrigger(null);
			} else {
				setMentionTrigger(null);
			}
			const nextSkillTrigger = findSkillTrigger(newValue, cursorPos);
			if (nextSkillTrigger) {
				setSkillTrigger(nextSkillTrigger);
				setSkillDismissed(false);
				setMentionTrigger(null);
			} else {
				setSkillTrigger(null);
			}

			const el = e.target;
			el.style.height = "auto";
			el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
		},
		[],
	);

	const handlePaste = useCallback(
		async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
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
				return;
			}
			const pastedText = e.clipboardData.getData?.("text/plain") ?? "";
			if (!pastedText) return;

			e.preventDefault();
			const el = e.currentTarget;
			const start = el.selectionStart ?? el.value.length;
			const end = el.selectionEnd ?? el.value.length;
			const nextIndex = pastedTextIdCounterRef.current + 1;
			try {
				const block = await invoke<PastedTextBlock | null>(
					"prepare_pasted_text_block",
					{
						index: nextIndex,
						content: pastedText,
					},
				);
				const insertion = block?.placeholder ?? pastedText;
				if (block) {
					pastedTextIdCounterRef.current = block.id;
					setPastedTextBlocks((current) => [...current, block]);
				}
				setSlashPopupDismissed(false);
				setSelectedIndex(0);
				setComposerValueWithCaret(
					`${el.value.slice(0, start)}${insertion}${el.value.slice(end)}`,
					start + insertion.length,
				);
			} catch (err) {
				console.error("Failed to prepare pasted text block:", err);
				setComposerValueWithCaret(
					`${el.value.slice(0, start)}${pastedText}${el.value.slice(end)}`,
					start + pastedText.length,
				);
			}
		},
		[addImages, setComposerValueWithCaret],
	);

	const canSend = value.trim().length > 0 || attachedImages.length > 0;
	const streamingSubmitLabel = supportsActiveTurnSteering
		? "Steer active turn"
		: "Queue message";
	const submitLabel = isStreaming ? streamingSubmitLabel : "Send message";
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
					placeholder={activePromptSuggestion ?? "Send a message..."}
					rows={1}
					className="w-full resize-none bg-transparent border-none px-3 pt-3 pb-1 text-sm focus:outline-none min-h-[36px] max-h-[200px]"
				/>
				{activeSlashArgumentHelp && (
					<div
						className="border-t px-3 py-1.5 text-xs text-muted-foreground"
						data-testid="slash-argument-help"
					>
						<span className="font-medium text-foreground">
							/{activeSlashArgumentHelp.name}
						</span>{" "}
						<span>{activeSlashArgumentHelp.argumentHint}</span>
						{activeSlashArgumentHelp.description ? (
							<span className="ml-2">
								{activeSlashArgumentHelp.description}
							</span>
						) : null}
					</div>
				)}
				<div className="flex items-center justify-between px-2 pb-2">
					<div className="flex items-center gap-1">
						<ModelSelector
							models={models}
							currentModelId={currentModelId}
							currentBackendId={currentBackendId}
							canChangeBackend={canChangeBackend}
							onModelChange={onModelChange}
							disabled={false}
						/>
						<ModeSelector
							mode={mode}
							onModeChange={onModeChange}
							disabled={false}
						/>
						<div
							className="flex h-7 items-center gap-1.5 px-2 text-xs select-none"
							title={planMode ? "Plan mode on" : "Plan mode off"}
						>
							<label htmlFor={planModeSwitchId} className="cursor-pointer">
								Plan
							</label>
							<Switch
								id={planModeSwitchId}
								checked={planMode}
								onCheckedChange={(checked) => onPlanModeChange(checked)}
								data-testid="plan-mode-toggle"
								aria-label="Plan mode"
							/>
						</div>
					</div>
					<div className="flex items-center gap-2">
						{isStreaming && canSend && (
							<span className="text-xs text-muted-foreground">
								{supportsActiveTurnSteering
									? "Steer active turn"
									: "Queue follow-up"}
							</span>
						)}
						{isStreaming && (
							<Button
								size="icon"
								variant="destructive"
								className="h-7 w-7 shrink-0"
								onClick={onInterrupt}
								disabled={isInterrupting}
								aria-label={
									isInterrupting ? "Stopping agent" : "Interrupt agent"
								}
								title={isInterrupting ? "Stopping…" : "Interrupt agent"}
							>
								{isInterrupting ? (
									<Loader2 className="size-3.5 animate-spin" />
								) : (
									<Square className="size-3.5" />
								)}
							</Button>
						)}
						{(!isStreaming || canSend) && (
							<Button
								size="icon"
								className="h-7 w-7 shrink-0"
								onClick={handleSubmit}
								disabled={!canSend || isSubmitting}
								aria-label={submitLabel}
								title={submitLabel}
							>
								<ArrowUp className="size-3.5" />
							</Button>
						)}
					</div>
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
			<SkillPopup
				open={skillPopupOpen}
				skills={skillCandidates}
				selectedIndex={skillSelectedIndex}
				onSelect={handleSelectSkill}
				onClose={() => setSkillDismissed(true)}
				anchorRef={inputRef}
			/>
		</>
	);
}
